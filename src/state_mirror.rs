use dashmap::DashMap;
use ethers::prelude::*;
use revm::primitives::Bytecode;
use crate::bindings::IUniswapV3Pool;
use ethers::abi::{self, Token};
use ethers::utils::hex;
use futures::future::join_all;
use arc_swap::ArcSwap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::mpsc::UnboundedSender;
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{error, warn, debug, info};
use crate::v3_math;
use crate::rpc_manager::RpcManager;
use rustc_hash::{FxHashMap, FxHashSet};

pub const MULTICALL_BATCH_SIZE: usize = 100; // Even larger batches to reduce RPC overhead
const MULTICALL_RETRIES: usize = 2;

#[derive(Clone, Debug)]
pub struct PoolState {
    pub reserves0: U256,
    pub reserves1: U256,
    pub liquidity: U256,
    pub sqrt_price_x96: U256,
    pub tick: i32,
    pub fee: u32, // For V3 pools
    pub dex_type: DexType,
    pub last_updated_block: u64,
    pub volatility_score: u64, // Pillar W: Volatility tracking for Wash-Trap Radar
    pub ticks: Arc<FxHashMap<i32, (i128, u128)>>, // Pillar F: Arc for O(1) cloning
    pub tick_bitmap: Arc<FxHashMap<i16, U256>>,   // word_pos -> bitmap
    pub last_swap_timestamp: u64, // Seconds since epoch
}

impl Default for PoolState {
    fn default() -> Self {
        Self {
            reserves0: U256::zero(),
            reserves1: U256::zero(),
            liquidity: U256::zero(),
            sqrt_price_x96: U256::zero(),
            tick: 0,
            fee: 0,
            dex_type: DexType::default(),
            last_updated_block: 0,
            volatility_score: 0,
            ticks: Arc::new(FxHashMap::default()),
            tick_bitmap: Arc::new(FxHashMap::default()),
            last_swap_timestamp: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum DexType {
    #[default]
    UniswapV2,
    UniswapV3,
    MaverickV2,
}

#[derive(Clone, Debug, Default)]
pub struct GasState {
    pub base_fee: U256,
    pub priority_fee: U256,
    pub next_base_fee: U256,
    pub max_priority_fee_per_gas: U256,
}

#[derive(Clone, Copy, Debug)]
enum CallType {
    GetReserves,
    Slot0,
    Liquidity,
    TickBitmap(i16),
    Ticks(i32),
    MaverickState,
}

pub struct StateMirror {
    pub pools: Arc<DashMap<Address, PoolState>>,
    pub bytecodes: Arc<DashMap<Address, Bytecode>>,
    pub gas_state: Arc<ArcSwap<GasState>>,
    pub dirty_flag: Arc<AtomicBool>,
    pub ws_provider: Arc<Provider<Ws>>,
    pub rpc_manager: Arc<RpcManager>, // Pillar T: Use HTTP for static calls
    pub multicall_address: Address,
    pub current_block: Arc<AtomicU64>,
    pub last_block_timestamp: Arc<AtomicU64>, // Pillar T: Anti-Drift Guardian
    pub sync_filter: Arc<ArcSwap<FxHashSet<Address>>>,
    pub p2p_priority_feed: Arc<AtomicBool>, // [P2P UPGRADE] Flag for direct sequencer feed
    pub poisoned_accounts: Arc<DashMap<Address, bool>>, // [OPTIMIZATION] Registry for Honeypots
}

impl StateMirror {
    pub fn new(ws_provider: Arc<Provider<Ws>>, rpc_manager: Arc<RpcManager>, multicall: Address) -> Arc<Self> {
        Arc::new(Self {
            pools: Arc::new(DashMap::new()),
            bytecodes: Arc::new(DashMap::new()),
            gas_state: Arc::new(ArcSwap::from_pointee(GasState::default())),
            dirty_flag: Arc::new(AtomicBool::new(true)),
            ws_provider,
            rpc_manager,
            multicall_address: multicall,
            current_block: Arc::new(AtomicU64::new(0)),
            last_block_timestamp: Arc::new(AtomicU64::new(0)),
            sync_filter: Arc::new(ArcSwap::from_pointee(FxHashSet::default())),
            p2p_priority_feed: Arc::new(AtomicBool::new(false)),
            poisoned_accounts: Arc::new(DashMap::new()),
        })
    }

    /// [PILLAR A+] Bypasses RPC Latency by consuming direct sequencer gossip.
    /// On Base Mainnet, this connects to the sequencer's broadcast endpoint.
    pub async fn spawn_p2p_gossip_handler(&self, p2p_url: String, trigger: Option<UnboundedSender<()>>) {
        info!("📡 [PILLAR A+] Connecting to Sequencer P2P Gossip Feed: {}", p2p_url);
        let current_block = self.current_block.clone();
        let priority_flag = self.p2p_priority_feed.clone();
        let dirty_flag = self.dirty_flag.clone();
        let last_block_timestamp = self.last_block_timestamp.clone();

        tokio::spawn(async move {
            // Pillar A+: Only attempt connection if URL is websocket
            if !p2p_url.starts_with("ws://") && !p2p_url.starts_with("wss://") {
                error!("❌ [P2P] Invalid URL scheme for P2P: {}. Must be ws:// or wss://", p2p_url);
                priority_flag.store(false, Ordering::SeqCst);
                return;
            }

            loop {
                match Provider::<Ws>::connect(&p2p_url).await {
                    Ok(stream) => {
                        info!("✅ [GHOST] Connected to Sequencer P2P Feed.");
                        priority_flag.store(true, Ordering::SeqCst);
                        if let Ok(mut sub) = stream.subscribe_blocks().await {
                            while let Some(block) = sub.next().await {
                                let new_num = block.number.unwrap_or_default().as_u64();
                                let old_num = current_block.swap(new_num, Ordering::AcqRel);
                                
                                let ts = block.timestamp.as_u64();
                                last_block_timestamp.store(ts, Ordering::Release);

                                if new_num > old_num {
                                    dirty_flag.store(true, Ordering::Release);
                                    if let Some(ref t) = trigger { let _ = t.send(()); }
                                    debug!("🐋 [P2P GOSSIP] Zero-latency trigger for block {}.", new_num);
                                }
                            }
                        }
                        warn!("📡 [P2P] Stream disconnected. Reconnecting...");
                    }
                    Err(e) => {
                        error!("❌ [P2P] Connection failed to {}: {}. FORCING fallback to standard WS/RPC.", p2p_url, e);
                        priority_flag.store(false, Ordering::SeqCst);
                        // Stop retrying to save throughput and allow standard sync to proceed
                        break; 
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        });
    }

    pub async fn sync(&self, external_block: Option<Block<H256>>) {
        if !self.dirty_flag.swap(false, Ordering::AcqRel) { return; }

        let block = match external_block {
            Some(b) => Some(b),
            None => {
                match self.rpc_manager.get_next_provider().get_block(BlockNumber::Latest).await {
                    Ok(b) => b,
                    Err(e) => { error!("[RADAR] sync block: {}", e); None }
                }
            }
        };

        if let Some(b) = block {
            let new_num = b.number.unwrap_or_default().as_u64();
            let old_num = self.current_block.swap(new_num, Ordering::AcqRel);
            self.last_block_timestamp.store(b.timestamp.as_u64(), Ordering::Release);

            if new_num < old_num && old_num > 0 {
                warn!("⚠️ [PILLAR T] Block Regression detected! Network: {} | Local: {}", new_num, old_num);
            }
            self.sync_gas_from_block(&b);
        }

        // Pillar CU: Only sync priority fee every 3 blocks to save Alchemy Compute Units
        if self.current_block.load(Ordering::Relaxed) % 3 == 0 {
            if let Err(e) = self.sync_priority_fee().await { error!("[RADAR] priority: {}", e); }
        }
        
        self.sync_pools_multicall().await;
    }

    fn sync_gas_from_block(&self, block: &Block<H256>) {
        let base_fee = block.base_fee_per_gas.unwrap_or_else(|| U256::from(20_000_000_000u64));
        let next_base_fee = (base_fee * 1125) / 1000;
        let current = self.gas_state.load();
        let mut new_gas = (**current).clone();
        new_gas.base_fee = base_fee;
        new_gas.next_base_fee = next_base_fee;
        self.gas_state.store(Arc::new(new_gas));
    }

    async fn sync_priority_fee(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let fee = self.rpc_manager.get_next_provider().get_gas_price().await?;
        let current = self.gas_state.load();
        let base_fee = current.base_fee;
        let suggested = if fee > base_fee { fee - base_fee } else { U256::from(1_500_000_000u64) };
        let mut new_gas = (**current).clone();
        let cap = new_gas.max_priority_fee_per_gas;
        new_gas.priority_fee = if cap > U256::zero() { std::cmp::min(suggested, cap) } else { suggested };
        debug!("[RADAR] priority fee: {} wei", new_gas.priority_fee);
        self.gas_state.store(Arc::new(new_gas));
        Ok(())
    }

    async fn sync_pools_multicall(&self) {
        if self.pools.is_empty() { return; }
        let start = Instant::now();
        let (all_calls, call_tracking) = self.build_multicall_calls();
        let mut tasks = Vec::new();
        for chunk_start in (0..all_calls.len()).step_by(MULTICALL_BATCH_SIZE) {
            let chunk_end = (chunk_start + MULTICALL_BATCH_SIZE).min(all_calls.len());
            let calls_chunk = Arc::new(all_calls[chunk_start..chunk_end].to_vec());
            let tracking_chunk = Arc::new(call_tracking[chunk_start..chunk_end].to_vec());
            
            let pools = Arc::clone(&self.pools);
            let provider = self.rpc_manager.get_next_provider();
            let multicall_address = self.multicall_address;
            let current_block = self.current_block.clone();

            tasks.push(tokio::spawn(async move {
                for attempt in 0..MULTICALL_RETRIES {
                    match Self::execute_multicall_raw(provider.clone(), multicall_address, calls_chunk.as_slice(), tracking_chunk.as_slice(), &pools, current_block.load(Ordering::Acquire)).await {
                        Ok(_) => break,
                        Err(e) => {
                            warn!("[RADAR] chunk fail {}/{}: {}", attempt+1, MULTICALL_RETRIES, e);
                            if attempt + 1 < MULTICALL_RETRIES {
                                tokio::time::sleep(std::time::Duration::from_millis(10)).await; // Reduced delay for faster recovery
                            }
                        }
                    }
                }
            }));
        }
        join_all(tasks).await;
        debug!("⚡ [SYNC] {} Pools Synced in {:?}", self.pools.len(), start.elapsed());
    }

    async fn execute_multicall_raw(
        provider: Arc<Provider<Http>>,
        multicall_address: Address,
        calls: &[Token],
        tracking: &[(Address, CallType)],
        pools: &Arc<DashMap<Address, PoolState>>,
        current_block: u64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    {
        let calldata = Self::encode_aggregate3(calls)?;
        let tx = TransactionRequest::new().to(multicall_address).data(calldata);
        let raw = provider.call(&tx.into(), None).await?;
        Self::decode_multicall_response_raw(raw, tracking, pools, current_block)
    }

    fn encode_aggregate3(calls: &[Token]) -> Result<Bytes, Box<dyn std::error::Error + Send + Sync>> {
        #[allow(deprecated)]
        let aggregate3_fn = ethers::abi::Function {
            name: "aggregate3".to_string(),
            inputs: vec![ethers::abi::Param {
                name: "calls".to_string(),
                kind: ethers::abi::ParamType::Array(Box::new(ethers::abi::ParamType::Tuple(vec![
                    ethers::abi::ParamType::Address,
                    ethers::abi::ParamType::Bool,
                    ethers::abi::ParamType::Bytes,
                ]))),
                internal_type: Some("tuple(address,bool,bytes)[]".to_string()),
            }],
            outputs: vec![],
            state_mutability: ethers::abi::StateMutability::View,
            constant: None,
        };
        Ok(aggregate3_fn.encode_input(&[Token::Array(calls.to_vec())])?.into())
    }

    fn decode_multicall_response_raw(raw: Bytes, tracking: &[(Address, CallType)], pools: &Arc<DashMap<Address, PoolState>>, current_block: u64)
        -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    {
        let return_type = ethers::abi::ParamType::Array(Box::new(
            ethers::abi::ParamType::Tuple(vec![
                ethers::abi::ParamType::Bool,
                ethers::abi::ParamType::Bytes,
            ])
        ));
        let decoded = abi::decode(&[return_type], &raw)?;
        let results = match decoded.get(0).and_then(|t| t.clone().into_array()) {
            Some(arr) => arr,
            None => return Err("Failed to decode multicall results array".into()),
        };

        if results.len() != tracking.len() {
            return Err(format!("multicall mismatch: {} vs {}", results.len(), tracking.len()).into());
        }

        for (i, result) in results.into_iter().enumerate() {
            let (addr, call_type) = tracking[i];
            let tuple = match result.into_tuple() {
                // If the multicall call itself failed (e.g., target contract doesn't exist)
                Some(t) => t,
                None => continue,
            };

            let success = tuple.get(0).and_then(|t| t.clone().into_bool()).unwrap_or(false);
            let ret = match tuple.get(1).and_then(|t| t.clone().into_bytes()) {
                Some(b) => b,
                None => continue,
            };

            if !success || ret.is_empty() { 
                debug!("❌ [SYNC] No data for pool {:?} - skipping", addr);
                continue; // Do not update last_updated_block if call failed
            }

            let mut entry = match pools.get_mut(&addr) {
                Some(e) => e,
                None => continue,
            };
            let s = entry.value_mut();

            let mut data_updated = false; // Flag to track if any data was successfully updated

            // [PILLAR F: ALPHA-CORE] Zero-Allocation Byte Slicing Decoding
            match call_type {
                CallType::GetReserves if ret.len() >= 64 => {
                    s.reserves0 = U256::from_big_endian(&ret[0..32]);
                    s.reserves1 = U256::from_big_endian(&ret[32..64]);
                    info!("✅ [RESERVES] Updated V2: {:?} | R0: {} | R1: {}", addr, s.reserves0, s.reserves1);
                    data_updated = true;
                }
                CallType::GetReserves => {
                    warn!("⚠️ [RESERVES] V2 Sync failed for {:?}: Data length mismatch (len: {})", addr, ret.len());
                }
                CallType::Slot0 if ret.len() >= 64 => {
                    s.sqrt_price_x96 = U256::from_big_endian(&ret[0..32]);
                    s.tick = i32::try_from(I256::from_raw(U256::from_big_endian(&ret[32..64]))).unwrap_or(0);
                    info!("✅ [RESERVES] Updated V3 Slot0: {:?} | Price: {}", addr, s.sqrt_price_x96);
                    data_updated = true;
                }
                CallType::Slot0 => {
                    warn!("⚠️ [RESERVES] V3 Slot0 for {:?} returned insufficient data (len: {}). Expected >= 64.", addr, ret.len());
                }
                CallType::Liquidity if ret.len() >= 32 => {
                    s.liquidity = U256::from_big_endian(&ret[0..32]);
                    info!("✅ [RESERVES] Updated V3 Liquidity for {:?}: Liq={}", addr, s.liquidity);
                    data_updated = true;
                }
                CallType::Liquidity => {
                    warn!("⚠️ [RESERVES] V3 Liquidity for {:?} returned insufficient data (len: {}). Expected >= 32.", addr, ret.len());
                }
                CallType::TickBitmap(word_pos) if ret.len() >= 32 => {
                    let bitmap = U256::from_big_endian(&ret[0..32]);
                    let mut new_bitmap = (*s.tick_bitmap).clone();
                    new_bitmap.insert(word_pos, bitmap);
                    s.tick_bitmap = Arc::new(new_bitmap);
                    data_updated = true;
                }
                CallType::Ticks(tick_idx) if ret.len() >= 64 => {
                    let gross = U256::from_big_endian(&ret[0..32]).as_u128();
                    let net = i128::try_from(I256::from_raw(U256::from_big_endian(&ret[32..64]))).unwrap_or(0);
                    let mut new_ticks = (*s.ticks).clone();
                    new_ticks.insert(tick_idx, (net, gross));
                    s.ticks = Arc::new(new_ticks);
                    data_updated = true;
                }
                CallType::MaverickState if ret.len() >= 96 => {
                    s.sqrt_price_x96 = U256::from_big_endian(&ret[0..32]);
                    s.tick = i32::try_from(I256::from_raw(U256::from_big_endian(&ret[32..64]))).unwrap_or(0);
                    s.liquidity = U256::from_big_endian(&ret[64..96]);
                    info!("✅ [RESERVES] Updated Maverick state for {:?}: SqrtPrice={}, Tick={}, Liq={}", addr, s.sqrt_price_x96, s.tick, s.liquidity);
                    data_updated = true;
                }
                CallType::MaverickState => {
                    warn!("⚠️ [RESERVES] MaverickState for {:?} returned insufficient data (len: {}). Expected >= 96.", addr, ret.len());
                }
                _ => {}
            }
            if data_updated {
                s.volatility_score = (current_block.saturating_sub(s.last_updated_block)) << 3; 
                s.last_updated_block = current_block;
            }
        }
        Ok(())
    }

    fn build_multicall_calls(&self) -> (Vec<Token>, Vec<(Address, CallType)>) {
        let filter = self.sync_filter.load();

        // V3: slot0, liq, 3 bitmaps, 3 ticks = 8 calls
        let cap = if filter.is_empty() {
            self.pools.len() * 8
        } else {
            filter.len() * 8
        };

        let mut calls = Vec::with_capacity(cap);
        let mut tracking = Vec::with_capacity(cap);
        for entry in self.pools.iter() {
            let (addr, state) = entry.pair();

            // Sync ALL registered pools — selective filter was causing Opps:0
        // let _ = &filter; // filter disabled intentionally

            // [LOGIC PURGE] Skip poisoned pools instantly
            if self.poisoned_accounts.contains_key(addr) { continue; }

            let target = *addr;
            match state.dex_type {
                DexType::UniswapV2 => {
                    calls.push(Token::Tuple(vec![
                        Token::Address(target),
                        Token::Bool(true),
                        // getReserves() selector
                        Token::Bytes(hex::decode("0902f1ac").expect("Static hex decode failed").to_vec()),
                    ]));
                    tracking.push((target, CallType::GetReserves));
                }
                DexType::UniswapV3 => {
                    let pool_contract = IUniswapV3Pool::new(target, self.ws_provider.clone());

                    if let Some(calldata) = pool_contract.slot_0().calldata() {
                        calls.push(Token::Tuple(vec![
                            Token::Address(target),
                            Token::Bool(true),
                            Token::Bytes(calldata.to_vec()),
                        ]));
                    }
                    tracking.push((target, CallType::Slot0));

                    if let Some(calldata) = pool_contract.liquidity().calldata() {
                        calls.push(Token::Tuple(vec![
                            Token::Address(target),
                            Token::Bool(true),
                            Token::Bytes(calldata.to_vec()),
                        ]));
                    }
                    tracking.push((target, CallType::Liquidity));

                    // Fetch tick bitmaps around the current tick
                    let (word_pos, _) = v3_math::tick_bitmap_position(state.tick);
                    for i in -1..=1 {
                        let pos = word_pos.saturating_add(i);
                        if let Some(calldata) = pool_contract.tick_bitmap(pos).calldata() {
                            calls.push(Token::Tuple(vec![
                                Token::Address(target), Token::Bool(true),
                                Token::Bytes(calldata.to_vec()),
                            ]));
                        }
                        tracking.push((target, CallType::TickBitmap(pos)));
                    }

                    // Fetch a few ticks around the current tick as a heuristic
                    let tick_spacing = v3_math::fee_to_tick_spacing(state.fee);
                    for i in -1..=1 {
                        let tick_to_fetch = state.tick.saturating_add(i * tick_spacing);
                        if let Some(calldata) = pool_contract.ticks(tick_to_fetch).calldata() {
                            calls.push(Token::Tuple(vec![
                                Token::Address(target), Token::Bool(true),
                                Token::Bytes(calldata.to_vec()),
                            ]));
                        }
                        tracking.push((target, CallType::Ticks(tick_to_fetch)));
                    }
                }
                DexType::MaverickV2 => {
                    // Pillar S: Maverick V2 getState() selector: 0x1ad855c2
                    calls.push(Token::Tuple(vec![
                        Token::Address(target),
                        Token::Bool(true),
                        Token::Bytes(hex::decode("1ad855c2").expect("Static hex failed").to_vec()),
                    ]));
                    tracking.push((target, CallType::MaverickState));
                }
            }
        }
        (calls, tracking)
    }

    pub fn get_pool_data(&self, address: &Address, max_age_blocks: u64) -> Option<PoolState> {
        self.pools.get(address).and_then(|p| {
            let current = self.current_block.load(Ordering::Acquire);
            // Accept pool if: never synced (last_updated_block=0) OR recently synced
            if p.last_updated_block == 0
                || current.saturating_sub(p.last_updated_block) <= max_age_blocks
            {
                Some(p.clone())
            } else { None }
        })
    }

    /// Pillar T: Anti-Drift Guardian
    /// Ensures our local state mirror is synchronized with the actual chain head.
    pub fn verify_state_freshness(&self) -> Result<(), crate::models::MEVError> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let block_ts = self.last_block_timestamp.load(Ordering::Acquire);
        
        if block_ts == 0 { return Ok(()); } // Initializing

        let drift = now.saturating_sub(block_ts);
        
        // Pillar T: Strict Latency Veto
        // Base par 2s drift ka matlab hai hum piche hain. 
        let limit = crate::constants::MAX_NODE_LAG_SECONDS;

        if drift > limit {
            return Err(crate::models::MEVError::Other(format!(
                "🛡️ [PILLAR T] Block Lag Guardian: State drift {}s > {}s limit. Sync is too slow for Base.", drift, limit
            )));
        }
        Ok(())
    }

    pub fn update_sync_filter(&self, filter: FxHashSet<Address>) {
        self.sync_filter.store(Arc::new(filter));
    }

    pub fn update_pool(&self, address: Address, state: PoolState) {
        self.pools.insert(address, state);
    }

    /// Fetch and cache bytecode for an address. Called during pool registration.
    pub async fn fetch_and_cache_bytecode(&self, address: Address) {
        if self.bytecodes.contains_key(&address) {
            return;
        }
        match self.rpc_manager.get_next_provider().get_code(address, None).await {
            Ok(code) if !code.0.is_empty() => {
                let bytecode = Bytecode::new_raw(code.0.into());

                self.bytecodes.insert(address, bytecode);
            }
            _ => {}
        }
    }

    pub fn get_bytecode(&self, address: &Address) -> Option<Bytecode> {
        self.bytecodes.get(address).map(|entry| entry.value().clone())
    }

    /// Quick check for poisoned addresses to keep the simulator hot-path clean.
    pub fn is_poisoned(&self, address: &Address) -> bool {
        self.poisoned_accounts.contains_key(address)
    }

    /// Pillar U: Advanced Memory Pruning.
    /// Removes pools and bytecodes that are stale or exceed RAM limits.
    pub fn prune_stale_pools(&self, max_age_blocks: u64) {
        let current_block = self.current_block_number();
        if current_block == 0 { return; }

        // 1. Prune stale pools by age — but never prune pools that haven't been synced yet
        self.pools.retain(|_addr, pool| {
            pool.last_updated_block == 0 // Keep unsynced pools
                || current_block.saturating_sub(pool.last_updated_block) <= max_age_blocks
        });

        // 2. Capacity Guard: Respecting Pillar U (8GB RAM target)
        if self.pools.len() > 5000 { // [MEMORY FIX] Lower threshold for 16Gi RAM
            warn!("🛡️ [PILLAR U] Pool cache overflow ({}). Evicting least active pools.", self.pools.len());
            // Keep only pools updated in the last 10 blocks to reclaim memory instantly
            self.pools.retain(|_, p| current_block.saturating_sub(p.last_updated_block) < 10);
        }

        // 3. Bytecode Pruning: Remove bytecodes for inactive pools to save RAM
        if self.bytecodes.len() > 1000 { // [MEMORY FIX] Strict bytecode limit
            self.bytecodes.retain(|addr, _| self.pools.contains_key(addr));
        }
    }

    pub fn mark_dirty(&self) {
        self.dirty_flag.store(true, Ordering::Release);
    }

    /// Lock-free reads via ArcSwap load.
    pub fn current_priority_fee(&self) -> U256 { self.gas_state.load().priority_fee }
    pub fn current_base_fee(&self) -> U256 { self.gas_state.load().base_fee }
    pub fn current_block_number(&self) -> u64 { self.current_block.load(Ordering::Acquire) }

    pub fn set_max_priority_fee(&self, max: U256) {
        let current = self.gas_state.load();
        let mut new_gas = (**current).clone();
        new_gas.max_priority_fee_per_gas = max;
        self.gas_state.store(Arc::new(new_gas));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_pool_state_default() { assert_eq!(PoolState::default().reserves0, U256::zero()); }
    #[test]
    fn test_gas_state_default() { assert_eq!(GasState::default().base_fee, U256::zero()); }
}
