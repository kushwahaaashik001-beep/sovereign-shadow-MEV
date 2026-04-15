#![allow(dead_code)]
use dashmap::{DashMap, DashSet};
use alloy_primitives::{Address, U256, Bytes};
use alloy::providers::Provider;
use alloy::transports::BoxTransport;
use alloy::sol;
use alloy::sol_types::SolCall;
use revm::primitives::{Bytecode, Bytes as rBytes};
use arc_swap::ArcSwap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use rustc_hash::FxHasher;
use std::hash::BuildHasherDefault;
use std::time::{SystemTime, UNIX_EPOCH};
use crate::models::DexType;
use rustc_hash::{FxHashMap, FxHashSet};

pub const MULTICALL_BATCH_SIZE: usize = 200; // Increased for Base Multicall3 efficiency

sol! {
    #[sol(rpc)]
    struct Call3 {
        address target;
        bool allowFailure;
        bytes callData;
    }

    #[sol(rpc)]
    struct MulticallResult {
        bool success;
        bytes returnData;
    }

    #[sol(rpc)]
    interface IMulticall3 {
        function aggregate3(Call3[] calldata calls) external payable returns (MulticallResult[] memory returnData);
    }

    interface IV2Pair {
        function getReserves() external view returns (uint112 reserve0, uint112 reserve1, uint32 blockTimestampLast);
    }

    interface IV3Pool {
        function slot0() external view returns (uint160 sqrtPriceX96, int24 tick, uint16 observationIndex, uint16 observationCardinality, uint16 observationCardinalityNext, uint8 feeProtocol, bool unlocked);
        function liquidity() external view returns (uint128);
    }
}

#[derive(Clone, Debug)]
pub struct PoolState {
    pub reserves0: U256,
    pub reserves1: U256,
    pub liquidity: U256,
    pub sqrt_price_x96: U256,
    pub tick: i32,
    pub fee: u32,
    pub dex_type: DexType,
    pub is_stable: bool,
    pub last_updated_block: u64,
    pub volatility_score: u64,
    pub ticks: Arc<FxHashMap<i32, (i128, u128)>>,
    pub tick_bitmap: Arc<FxHashMap<i16, U256>>,
    pub last_swap_timestamp: u64,
}

impl Default for PoolState {
    fn default() -> Self {
        Self {
            reserves0: U256::ZERO,
            reserves1: U256::ZERO,
            liquidity: U256::ZERO,
            sqrt_price_x96: U256::ZERO,
            tick: 0,
            fee: 0,
            dex_type: DexType::default(),
            is_stable: false,
            last_updated_block: 0,
            volatility_score: 0,
            ticks: Arc::new(FxHashMap::default()),
            tick_bitmap: Arc::new(FxHashMap::default()),
            last_swap_timestamp: 0,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct GasState {
    pub base_fee: U256,
    pub priority_fee: U256,
    pub next_base_fee: U256,
    pub max_priority_fee_per_gas: U256,
}

impl GasState {
    pub fn current_fees(&self) -> (U256, U256, U256) {
        (self.base_fee, self.priority_fee, self.max_priority_fee_per_gas)
    }
}

pub struct StateMirror {
    pub pools: Arc<DashMap<Address, PoolState, BuildHasherDefault<FxHasher>>>,
    pub bytecodes: Arc<DashMap<Address, Bytecode, BuildHasherDefault<FxHasher>>>,
    pub gas_state: Arc<ArcSwap<GasState>>,
    pub dirty_flag: Arc<AtomicBool>,
    pub current_block: Arc<AtomicU64>,
    pub last_block_timestamp: Arc<AtomicU64>,
    pub last_multicall_sync: Arc<AtomicU64>,
    pub storage_cache: Arc<DashMap<(Address, U256), U256, BuildHasherDefault<FxHasher>>>, // Added storage tracking
    pub sync_filter: Arc<ArcSwap<FxHashSet<Address>>>,
    pub p2p_priority_feed: Arc<AtomicBool>,
    pub poisoned_accounts: Arc<DashMap<Address, bool, BuildHasherDefault<FxHasher>>>,
    pub trader_registry: Arc<DashMap<Address, DashSet<Address, BuildHasherDefault<FxHasher>>, BuildHasherDefault<FxHasher>>>,
}

impl StateMirror {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            pools: Arc::new(DashMap::with_hasher(BuildHasherDefault::default())),
            bytecodes: Arc::new(DashMap::with_hasher(BuildHasherDefault::default())),
            gas_state: Arc::new(ArcSwap::from_pointee(GasState::default())),
            dirty_flag: Arc::new(AtomicBool::new(true)),
            current_block: Arc::new(AtomicU64::new(0)),
            last_block_timestamp: Arc::new(AtomicU64::new(0)),
            last_multicall_sync: Arc::new(AtomicU64::new(0)),
            storage_cache: Arc::new(DashMap::with_hasher(BuildHasherDefault::default())),
            sync_filter: Arc::new(ArcSwap::from_pointee(FxHashSet::default())),
            p2p_priority_feed: Arc::new(AtomicBool::new(false)),
            poisoned_accounts: Arc::new(DashMap::with_hasher(BuildHasherDefault::default())),
            trader_registry: Arc::new(DashMap::with_hasher(BuildHasherDefault::default())),
        })
    }

    pub async fn sync_block(&self, block_number: u64, base_fee: U256, timestamp: u64) {
        let prev = self.current_block.load(Ordering::Acquire);
        if prev != 0 && block_number > prev + 1 {
            tracing::warn!("⚠️ [STATE] Gap detected: Missed {} blocks. Forcing resync.", block_number - prev - 1);
            self.mark_dirty();
        }
        
        self.current_block.store(block_number, Ordering::Release);
        self.last_block_timestamp.store(timestamp, Ordering::Release);
        
        let next_base_fee = (base_fee * U256::from(1125u64)) / U256::from(1000u64); // Standard EIP-1559 12.5% increase
        let current = self.gas_state.load();
        let mut new_gas = (**current).clone();
        new_gas.base_fee = base_fee;
        new_gas.next_base_fee = next_base_fee;
        self.gas_state.store(Arc::new(new_gas));
    }

    pub fn verify_state_freshness(&self) -> Result<(), crate::models::MEVError> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let block_ts = self.last_block_timestamp.load(Ordering::Acquire);
        if block_ts == 0 { return Ok(()); }
        let drift = now.saturating_sub(block_ts);
        let limit = crate::constants::MAX_NODE_LAG_SECONDS;
        if drift > limit {
            return Err(crate::models::MEVError::Other(format!(
                "🛡️ [PILLAR T] Block Lag: {}s > {}s limit", drift, limit
            )));
        }
        Ok(())
    }

    /// Pillar W: Record a unique trader for a pool to detect wash-trading.
    pub fn record_trader(&self, pool: Address, trader: Address) {
        if trader.is_zero() { return; }
        let entry = self.trader_registry.entry(pool).or_insert_with(|| {
            DashSet::with_hasher(BuildHasherDefault::default())
        });
        entry.insert(trader);
        
        // Memory Management: Keep registry lean
        if entry.len() > 100 { entry.clear(); } 
    }

    pub fn update_sync_filter(&self, filter: FxHashSet<Address>) {
        self.sync_filter.store(Arc::new(filter));
    }

    pub fn update_pool(&self, address: Address, state: PoolState) {
        self.pools.insert(address, state);
    }

    /// Pillar B/H: Fetch bytecode from RPC and scan for malicious patterns.
    pub async fn fetch_and_cache_bytecode<P: Provider<BoxTransport>>(&self, address: Address, provider: Arc<P>) {
        // Skip if address is a known safe token (WETH, USDC etc)
        if crate::constants::CORE_TOKENS.contains(&address) {
            return;
        }

        if self.bytecodes.contains_key(&address) || self.is_poisoned(&address) {
            return;
        }

        match provider.get_code_at(address).await {
            Ok(code) => {
                if code.is_empty() { return; }
                
                // Pillar H: Static Analysis for Honeypot Opcodes
                if self.is_malicious_bytecode(&code) {
                    tracing::warn!("🛡️ [PILLAR H] Malicious pattern detected at {:?}", address);
                    self.poisoned_accounts.insert(address, true);
                    return;
                }

                let revm_code = Bytecode::new_raw(rBytes::from(code.to_vec()));
                self.bytecodes.insert(address, revm_code);
            }
            Err(e) => tracing::error!("❌ [State Mirror] Bytecode fetch error: {}", e),
        }
    }

    fn is_malicious_bytecode(&self, code: &Bytes) -> bool {
        let code_ref = code.as_ref();

        // 1. Known Malicious Signatures Match
        for sig in crate::constants::HONEYPOT_BYTECODE_SIGNATURES.iter() {
            if code_ref.windows(sig.len()).any(|window| window == sig) {
                return true;
            }
        }

        // 2. Pillar X: X-Ray Scanner - Deep Opcode Analysis
        // Uses the MALICIOUS_OPCODES set to scan for dangerous instructions.
        for &opcode in crate::constants::MALICIOUS_OPCODES.iter() {
            if code_ref.contains(&opcode) {
                // Special Case: DELEGATECALL (0xf4) is common in high-quality proxies.
                // We only flag it if the bytecode is suspiciously small (< 5000 bytes).
                if opcode == 0xf4 && code_ref.len() >= 5000 { continue; }
                return true;
            }
        }

        // 3. Blacklist Pattern Detection (CALLER + SLOAD + REVERT)
        // यह उन टोकन्स को पकड़ता है जो खास एड्रेस (जैसे हमारा बॉट) के लिए ट्रांसफर रोक देते हैं।
        let has_caller = code_ref.contains(&0x33); // CALLER
        let has_sload = code_ref.contains(&0x54);  // SLOAD
        let has_revert = code_ref.contains(&0xfd) || code_ref.contains(&0xfe);

        if has_caller && has_sload && has_revert && code_ref.len() < 3000 {
            return true;
        }

        false
    }

    /// Pillar B: Sub-block Batch Sync Logic.
    /// Synchronizes all 3000+ pools in massive batches using Multicall3.
    pub async fn sync_all_pools_multicall<P: Provider<BoxTransport>>(&self, provider: Arc<P>) -> Result<(), crate::models::MEVError> {
        let pool_addresses: Vec<(Address, DexType)> = self.pools.iter().map(|entry| (*entry.key(), entry.value().dex_type)).collect::<Vec<_>>();
        if pool_addresses.is_empty() { return Ok(()); }

        let multicall_addr = alloy_primitives::address!("0xcA11bde05977b3631167028862bE2a173976CA11");
        let multicall = IMulticall3::IMulticall3Instance::new(multicall_addr, provider.clone());
        let current_block = self.current_block_number();

        for chunk in pool_addresses.chunks(MULTICALL_BATCH_SIZE) {
            let mut calls = Vec::with_capacity(chunk.len() * 2);
            
            for (addr, dex_type) in chunk {
                match dex_type {
                    DexType::UniswapV2 | DexType::Aerodrome => {
                        calls.push(Call3 {
                            target: *addr,
                            allowFailure: true,
                            callData: IV2Pair::getReservesCall {}.abi_encode().into(),
                        });
                    }
                    DexType::UniswapV3 | DexType::MaverickV2 => {
                        calls.push(Call3 {
                            target: *addr,
                            allowFailure: true,
                            callData: IV3Pool::slot0Call {}.abi_encode().into(),
                        });
                        calls.push(Call3 {
                            target: *addr,
                            allowFailure: true,
                            callData: IV3Pool::liquidityCall {}.abi_encode().into(),
                        });
                    }
                }
            }

            if let Ok(output) = multicall.aggregate3(calls).call().await {
                let mut results_iter = output.returnData.into_iter();
                let mut updates = Vec::with_capacity(chunk.len());

                for (addr, dex_type) in chunk {
                    let mut state = PoolState { last_updated_block: current_block, dex_type: *dex_type, ..Default::default() };
                    
                    match dex_type {
                        DexType::UniswapV2 | DexType::Aerodrome => {
                            if let Some(res) = results_iter.next() {
                                if res.success {
                                    if let Ok(decoded) = IV2Pair::getReservesCall::abi_decode_returns(&res.returnData.0, true) {
                                        state.reserves0 = U256::from(decoded.reserve0);
                                        state.reserves1 = U256::from(decoded.reserve1);
                                        updates.push((*addr, state));
                                    }
                                }
                            }
                        }
                        DexType::UniswapV3 | DexType::MaverickV2 => {
                            let slot0_res = results_iter.next();
                            let liq_res = results_iter.next();
                            
                            if let (Some(s0), Some(l)) = (slot0_res, liq_res) {
                                if s0.success && l.success {
                                    let s0_dec = IV3Pool::slot0Call::abi_decode_returns(&s0.returnData.0, true);
                                    let l_dec = IV3Pool::liquidityCall::abi_decode_returns(&l.returnData.0, true);
                                    
                                    if let (Ok(s), Ok(liq)) = (s0_dec, l_dec) {
                                        state.sqrt_price_x96 = U256::from(s.sqrtPriceX96);
                                        state.tick = s.tick.as_i32();
                                        state.liquidity = U256::from(liq._0);
                                        updates.push((*addr, state));
                                    }
                                }
                            }
                        }
                    }
                }
                self.batch_update_reserves(updates);
            }
        }

        self.last_multicall_sync.store(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(), Ordering::Release);
        Ok(())
    }

    pub fn get_bytecode(&self, address: &Address) -> Option<Bytecode> {
        self.bytecodes.get(address).map(|e: dashmap::mapref::one::Ref<'_, Address, Bytecode, _>| e.value().clone())
    }

    pub fn is_poisoned(&self, address: &Address) -> bool {
        self.poisoned_accounts.contains_key(address)
    }

    pub fn prune_stale_pools(&self, max_age_blocks: u64) {
        let current = self.current_block_number();
        if current == 0 { return; }
        self.pools.retain(|_, pool| {
            // Remove if no activity for max_age_blocks
            // If last_updated is 0, it means it's a freshly discovered pool but never traded.
            // We give it a 100 block grace period before purging.
            let age = if pool.last_updated_block == 0 { 0 } else { current.saturating_sub(pool.last_updated_block) };
            age <= max_age_blocks
        });
        if self.pools.len() > 3000 {
            self.pools.retain(|_, p| current.saturating_sub(p.last_updated_block) < 10);
        }
        if self.bytecodes.len() > 500 {
            self.bytecodes.retain(|addr, _| self.pools.contains_key(addr));
        }
    }

    pub fn mark_dirty(&self) {
        self.dirty_flag.store(true, Ordering::Release);
    }

    pub fn current_priority_fee(&self) -> U256 { self.gas_state.load().priority_fee }
    pub fn current_base_fee(&self) -> U256 { self.gas_state.load().base_fee }
    pub fn current_block_number(&self) -> u64 { self.current_block.load(Ordering::Acquire) }

    pub fn set_max_priority_fee(&self, max: U256) {
        let current = self.gas_state.load();
        let mut new_gas = (**current).clone();
        new_gas.max_priority_fee_per_gas = max;
        self.gas_state.store(Arc::new(new_gas));
    }

    pub fn get_pool_data(&self, address: &Address, max_age_blocks: u64) -> Option<PoolState> {
        self.pools.get(address).and_then(|p| {
            let current = self.current_block.load(Ordering::Acquire);
            if p.last_updated_block == 0 || current.saturating_sub(p.last_updated_block) <= max_age_blocks {
                Some(p.clone())
            } else { None }
        })
    }

    /// Pillar B: Update multiple pool reserves in a single block using Multicall data.
    pub fn batch_update_reserves(&self, updates: Vec<(Address, PoolState)>) {
        for (addr, state) in updates {
            self.pools.entry(addr)
                .and_modify(|existing| {
                    existing.reserves0 = state.reserves0;
                    existing.reserves1 = state.reserves1;
                    existing.last_updated_block = state.last_updated_block;
                    
                    // V3 fields
                    if state.sqrt_price_x96 != U256::ZERO {
                        existing.sqrt_price_x96 = state.sqrt_price_x96;
                        existing.tick = state.tick;
                        existing.liquidity = state.liquidity;
                    }

                    // Pillar Z: Aerodrome Stable Flag persistence from Multicall
                    if state.is_stable {
                        existing.is_stable = true;
                        existing.dex_type = DexType::Aerodrome;
                    }
                })
                .or_insert(state);
        }
        self.mark_dirty();
    }

    pub fn update_v2_reserves(&self, address: Address, r0: U256, r1: U256) {
        self.pools.entry(address)
            .and_modify(|p| {
                p.reserves0 = r0;
                p.reserves1 = r1;
                p.last_updated_block = self.current_block_number();
            });
        self.mark_dirty();
    }

    pub fn update_v3_state(&self, address: Address, sqrt_price: U256, tick: i32, liquidity: U256) {
        self.pools.entry(address)
            .and_modify(|p| {
                p.sqrt_price_x96 = sqrt_price;
                p.tick = tick;
                p.liquidity = liquidity;
                p.last_updated_block = self.current_block_number();
            });
        self.mark_dirty();
    }

    pub fn update_aerodrome_stable(&self, address: Address, is_stable: bool) {
        self.pools.entry(address)
            .and_modify(|p| {
                p.is_stable = is_stable;
                p.dex_type = DexType::Aerodrome;
                p.last_updated_block = self.current_block_number();
            });
        self.mark_dirty();
    }
}

impl revm::DatabaseRef for StateMirror {
    type Error = crate::models::MEVError;

    fn basic_ref(&self, address: Address) -> Result<Option<revm::primitives::AccountInfo>, Self::Error> {
        // Zero-Copy: Fetching bytecode reference from DashMap
        if let Some(code) = self.get_bytecode(&address) {
            Ok(Some(revm::primitives::AccountInfo {
                code_hash: code.hash_slow(),
                code: Some(code),
                ..Default::default()
            }))
        } else {
            Ok(None)
        }
    }

    fn code_by_hash_ref(&self, _code_hash: alloy_primitives::B256) -> Result<Bytecode, Self::Error> {
        Err(crate::models::MEVError::Other("Use basic_ref".into()))
    }

    #[inline(always)]
    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        // Nanosecond Optimization: Use get_key_value to avoid double hashing
        Ok(self.storage_cache.get(&(address, index))
            .map(|v: dashmap::mapref::one::Ref<'_, (Address, U256), U256, _>| *v.value()).unwrap_or(U256::ZERO))
    }

    fn block_hash_ref(&self, _number: u64) -> Result<alloy_primitives::B256, Self::Error> {
        Ok(alloy_primitives::B256::ZERO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_pool_state_default() { assert_eq!(PoolState::default().reserves0, U256::ZERO); }
    #[test]
    fn test_gas_state_default() { assert_eq!(GasState::default().base_fee, U256::ZERO); }
}
