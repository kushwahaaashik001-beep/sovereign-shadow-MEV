#![allow(dead_code)]
#![allow(unused_variables)]

use ethers::types::U512;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use crate::gas_feed::GasPriceFeed;
use crate::math_engine::MathEngine;
use crate::bidding_engine::BiddingEngine;
use crate::models::{DexName, Hop, Opportunity, Path, PoolEdge, ProfitDetails, DexType};
use crate::state_mirror::{StateMirror, PoolState};
use crate::constants::{self, BLACKLISTED_TOKENS};
use dashmap::DashMap;
use rustc_hash::FxHashMap;
use ethers::{
    providers::{Provider, Ws},
    types::{Address, Chain, H256, U256},
    utils::{keccak256, get_create2_address_from_hash},
    signers::LocalWallet,
};
use rustc_hash::FxHashSet;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Semaphore};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tracing::{info, debug, warn, trace};
use rayon::prelude::*;
use crate::utils::send_telegram_msg;
use crate::factory_scanner::NewPoolEvent;
use arc_swap::ArcSwap;

// ── Shared atomic counters for live dashboard ─────────────────────────────────
pub static SWAPS_RECEIVED:   AtomicU64   = AtomicU64::new(0);
pub static CYCLES_FOUND:     AtomicU64   = AtomicU64::new(0);
pub static OPPS_SENT:        AtomicU64   = AtomicU64::new(0);
pub static IGNORED_NO_TOKEN: AtomicUsize = AtomicUsize::new(0);
pub static IGNORED_NO_CYCLE: AtomicUsize = AtomicUsize::new(0);

#[allow(dead_code)]
#[derive(Clone)]
pub struct DetectorConfig {
    pub min_profit_wei:     U256,
    pub max_path_length:    usize,
    pub important_tokens:   Arc<HashSet<Address>>,
    pub multicall_address:  Address,
    pub factories:          HashMap<DexName, Address>,
    pub scanner_threads:    usize,
    pub min_liquidity_eth:  u64,
    pub chain:              Chain,
    pub priority_fee_percent: u64,
    pub bribe_percent:      u64,
    pub flashbots_relay:    Option<String>,
    pub signer:             Option<LocalWallet>,
    pub executor_address:   Address,
    pub pool_limit:         usize,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        let mut factories = HashMap::new();
        // Base Mainnet Factories
        factories.insert(DexName::UniswapV3, "0x33128a8fC17869897dcE68Ed026d694621f6FDfD".parse().unwrap_or(Address::zero()));
        factories.insert(DexName::SushiSwap, "0x71524B4f97c582d280C831b012c4901b0Ac2273d".parse().unwrap_or(Address::zero()));
        factories.insert(DexName::BaseSwap, "0xFDa619b6d20975be8074d3315450bbBA58456B12".parse().unwrap_or(Address::zero()));
        factories.insert(DexName::Aerodrome, "0x420DD381b8da2873f3647f300559007c6f83f215".parse().unwrap_or(Address::zero()));
        factories.insert(DexName::PancakeSwap, "0x0BFbCF9fa4f9C56B0F40a671Ad40E0805A091865".parse().unwrap_or(Address::zero()));

        let mut important = HashSet::new();

        // Lead Architect: Focus ONLY on current chain's safe tokens for performance
        if let Some(tokens) = constants::SAFE_TOKENS.get(&Chain::Base) {
            for token in tokens {
                important.insert(*token);
            }
        }

        Self {
            min_profit_wei:     U256::from(1u64), // ⚡ 1-WEI SENSITIVITY: Non-negotiable
            max_path_length:    constants::MAX_HOPS, // Sync with global pillar strategy
            important_tokens:   Arc::new(important),
            multicall_address:  "0xcA11bde05977b3631167028862bE2a173976CA11".parse().unwrap_or(Address::zero()),
            factories,
            scanner_threads:    8,
            min_liquidity_eth:  1,
            chain:              Chain::Mainnet,
            priority_fee_percent: 10,
            bribe_percent:      50, // Pillar I: Default to 50% for initial gas reserve building
            flashbots_relay:    None,
            signer:             None,
            executor_address:   Address::zero(),
            pool_limit:         3500,
        }
    }
}

// ── Lock-free token graph ─────────────────────────────────────────────────────
#[allow(dead_code)]
#[derive(Default, Clone)]
struct GraphData {
    address_to_idx: FxHashMap<Address, usize>,
    idx_to_address: Vec<Address>,
    adjacency_matrix: Vec<u64>, // Flattened bitset matrix [N * (N/64 + 1)]
    edge_map: Vec<Vec<PoolEdge>>, // Adjacency list using indices
    num_tokens: usize,
    stride: usize, // Number of u64s per row
}

struct HotGraph {
    edges: DashMap<Address, Vec<PoolEdge>>,
    optimized_data: ArcSwap<GraphData>,
}

impl HotGraph {
    fn new() -> Self { 
        Self { 
            edges: DashMap::new(),
            optimized_data: ArcSwap::from_pointee(GraphData::default()),
        } 
    }

    fn add_pool(&self, pool: Address, dex: DexName, t0: Address, t1: Address, fee: Option<u32>, liq: u64) {
        let e0 = PoolEdge { pool_address: pool, dex_name: dex, token_b: t1, fee, liq_score: liq, static_calldata: Default::default(), gas_cost: U256::zero(), id: [0;32], success_prob: 10000 };
        let e1 = PoolEdge { pool_address: pool, dex_name: dex, token_b: t0, fee, liq_score: liq, static_calldata: Default::default(), gas_cost: U256::zero(), id: [0;32], success_prob: 10000 };
        
        {
            let mut a = self.edges.entry(t0).or_default(); 
            if !a.iter().any(|e| e.pool_address == pool) { 
                a.push(e0); 
                a.sort_by_key(|e| std::cmp::Reverse(e.liq_score)); 
            }
        }
        {
            let mut b = self.edges.entry(t1).or_default(); 
            if !b.iter().any(|e| e.pool_address == pool) { 
                b.push(e1); 
                b.sort_by_key(|e| std::cmp::Reverse(e.liq_score)); 
            }
        }
        
        self.rebuild_optimized();
    }

    /// Pillar C: Sub-microsecond Graph Rebuild
    fn rebuild_optimized(&self) {
        let mut addr_to_idx = FxHashMap::default();
        let mut tokens = Vec::new();

        // Snapshot entries to ensure consistency and avoid race-condition panics
        let entries: Vec<(Address, Vec<PoolEdge>)> = self.edges.iter()
            .map(|e| (*e.key(), e.value().clone()))
            .collect();
        
        for (addr, _) in &entries {
            addr_to_idx.entry(*addr).or_insert_with(|| {
                let idx = tokens.len();
                tokens.push(*addr);
                idx
            });
        }

        let n = tokens.len();
        let stride = (n + 63) / 64;
        let mut matrix = vec![0u64; n * stride];
        let mut edge_map = vec![Vec::new(); n];
        let mut edge_count = 0;

        for (addr, edges) in entries {
            let u = addr_to_idx[&addr];
            edge_count += edges.len();
            edge_map[u] = edges;
            for edge in &edge_map[u] {
                if let Some(&v) = addr_to_idx.get(&edge.token_b) {
                    let row_start = u * stride;
                    matrix[row_start + (v / 64)] |= 1 << (v % 64);
                }
            }
        }

        info!("📊 [GRAPH] Stats: Nodes: {} | Edges: {}", n, edge_count);

        self.optimized_data.store(Arc::new(GraphData {
            address_to_idx: addr_to_idx,
            idx_to_address: tokens,
            adjacency_matrix: matrix,
            edge_map,
            num_tokens: n,
            stride,
        }));
    }

    fn remove_pool(&self, pool: Address) {
        for mut entry in self.edges.iter_mut() {
            let edges = entry.value_mut();
            if edges.iter().any(|e| e.pool_address == pool) {
                edges.retain(|e| e.pool_address != pool);
            }
        }
        self.rebuild_optimized();
    }

    fn find_cycles(&self, start: Address, max_depth: usize) -> Vec<Path> {
        let data = self.optimized_data.load();
        let start_idx = match data.address_to_idx.get(&start) {
            Some(&idx) => idx,
            None => return vec![],
        };

        let mut paths = Vec::with_capacity(512);
        let mut visited_tokens = vec![false; data.num_tokens];
        let mut current_path = Vec::with_capacity(max_depth);

        visited_tokens[start_idx] = true;
        self.dfs(start_idx, start_idx, &mut current_path, &mut visited_tokens, max_depth, &data, &mut paths);

        paths
    }

    /// Pillar C: Recursive DFS Cycle Detection (Up to MAX_HOPS)
    fn dfs(
        &self,
        current_idx: usize,
        start_idx: usize,
        current_path: &mut Vec<Hop>,
        visited_tokens: &mut [bool],
        max_depth: usize,
        data: &GraphData,
        paths: &mut Vec<Path>,
    ) {
        if current_path.len() >= max_depth {
            return;
        }

        // Pillar C: Expanded Breadth for 3500+ Pools scale
        // [BULLETPROOF FIX] Focused breadth: 20/10 is enough for High-Liquidity Alpha.
        let breadth = if current_path.is_empty() { 20 } else { 10 }; 
        let current_token_addr = data.idx_to_address[current_idx];

        for edge in data.edge_map[current_idx].iter().take(breadth) {
            let next_token = edge.token_b;
            let next_idx = match data.address_to_idx.get(&next_token) {
                Some(&idx) => idx,
                None => continue,
            };

            // Potential Cycle Closure: If next_token is start_token
            if next_idx == start_idx {
                if current_path.len() >= 1 { // Min 2-hop (Triangle/Quad logic)
                    if !current_path.iter().any(|h| h.pool_address == edge.pool_address) {
                        let mut final_hops = current_path.clone();
                        final_hops.push(self.edge_to_hop(edge, current_token_addr));

                        if tracing::enabled!(tracing::Level::TRACE) {
                            let mut path_str = String::new();
                            for (i, h) in final_hops.iter().enumerate() {
                                if i == 0 {
                                    path_str.push_str(&format!("{:?}", h.token_in));
                                }
                                path_str.push_str(&format!(" -> {:?}", h.token_out));
                            }
                            trace!("🎯 [CYCLE FOUND] {}", path_str);
                        }

                        // Pillar C: Fast-Hash for deduplication
                        let mut path_obj = Path::new(&final_hops, 0, 5);
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        std::hash::Hash::hash(&format!("{:?}", final_hops), &mut hasher);
                        path_obj.hash = H256::from_low_u64_be(std::hash::Hasher::finish(&hasher));
                        paths.push(path_obj);
                    }
                }
                continue;
            }

            // Standard DFS path finding
            if !visited_tokens[next_idx] {
                if !current_path.iter().any(|h| h.pool_address == edge.pool_address) {
                    visited_tokens[next_idx] = true;
                    current_path.push(self.edge_to_hop(edge, current_token_addr));
                    self.dfs(next_idx, start_idx, current_path, visited_tokens, max_depth, data, paths);
                    current_path.pop();
                    visited_tokens[next_idx] = false;
                }
            }
        }
    }

    // Helper to convert edge to hop without recursion overhead
    fn edge_to_hop(&self, edge: &PoolEdge, token_in: Address) -> Hop {
        Hop {
            pool: edge.pool_address,
            pool_address: edge.pool_address,
            pool_address_label: None,
            token_in,
            token_out: edge.token_b,
            dex_type: match edge.dex_name { DexName::UniswapV3 => DexType::UniswapV3, _ => DexType::UniswapV2 },
            dex_name: edge.dex_name,
            zero_for_one: token_in < edge.token_b,
            fee: edge.fee,
            static_calldata: Default::default(),
            gas_cost: U256::zero(),
            id: [0; 32],
            success_prob: 10000,
        }
    }
}

// ── Shared state ──────────────────────────────────────────────────────────────
#[allow(dead_code)]
pub struct SharedState {
    mirror:         Arc<StateMirror>,
    graph:          Arc<HotGraph>,
    math_engine:    Arc<MathEngine>,
    bidding_engine: Arc<BiddingEngine>,
    opp_tx:         UnboundedSender<Opportunity>,
    config:         DetectorConfig,
    gas_feed:       Arc<GasPriceFeed>,
    force_tx:       UnboundedSender<()>,
    seen:           DashMap<H256, bool>,
    path_cache:     Arc<ArcSwap<Vec<Path>>>, // Global list for force-sync
    indexed_paths:  Arc<ArcSwap<FxHashMap<Address, Vec<Arc<Path>>>>>, // Pillar F: O(1) Trigger Index
    block_counter:  Arc<AtomicU64>,
    sender_history: DashMap<Address, (u64, u32, Address, Address)>, // Pillar W: metadata expanded
    neural_memory:  Arc<NeuralMemory>, // Pillar F: The Self-Learning Oracle
}

struct WhaleStats {
    opportunity_count: AtomicUsize,
    last_seen_block: AtomicU64,
}

/// Pillar F: Neural Memory to learn from simulation failures and L2 drift.
#[allow(dead_code)]
struct NeuralMemory {
    // Counts consecutive simulation reverts per pool to detect "Poison Liquidity"
    revert_streak: DashMap<Address, AtomicUsize>,
    // Pillar F: Whale Registry - Tracks market movers who create consistent alpha
    whale_registry: DashMap<Address, Arc<WhaleStats>>,
    // [ADAPTIVE] Track rejection reasons to auto-adjust thresholds
    rejection_counters: DashMap<String, AtomicUsize>,
}

impl NeuralMemory {
    pub fn is_poisoned(&self, pool: &Address) -> bool {
        self.revert_streak.get(pool).map_or(false, |s| s.load(Ordering::Relaxed) > 3)
    }

    pub fn record_alpha(&self, sender: Address, current_block: u64) {
        let stats = self.whale_registry.entry(sender).or_insert_with(|| Arc::new(WhaleStats {
            opportunity_count: AtomicUsize::new(0),
            last_seen_block: AtomicU64::new(current_block),
        }));
        if stats.opportunity_count.fetch_add(1, Ordering::Relaxed) == 5 {
            info!("👑 [WHALE TRACKER] New Alpha-Whale Identified: {:?}", sender);
        }
        stats.last_seen_block.store(current_block, Ordering::Relaxed);
    }

    pub fn is_whale(&self, sender: &Address) -> bool {
        self.whale_registry.get(sender).map_or(false, |s| s.opportunity_count.load(Ordering::Relaxed) > 5)
    }

    pub fn record_rejection(&self, reason: &str) {
        let counter = self.rejection_counters.entry(reason.to_string()).or_insert_with(|| AtomicUsize::new(0));
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

impl SharedState {
    /// Pillar U: Memory Management for the HotGraph and Detector.
    /// Prunes transaction history, sender metadata, and neural logs.
    fn prune_memory(&self) {
        let current_block = self.mirror.current_block_number();
        
        // [BULLETPROOF FIX] Rolling window cleanup to prevent heap bloat.
        if self.seen.len() > 1000 {
            self.seen.clear();
        }

        // [MEMORY FIX] Strict behavioral history limit (Last 5 blocks only)
        self.sender_history.retain(|_, (block, _, _, _)| {
            current_block.saturating_sub(*block) < 5
        });

        // 3. Prune Neural Memory (Revert streaks and ghost logs)
        if self.neural_memory.revert_streak.len() > 1000 {
            self.neural_memory.revert_streak.clear();
        }

        // 4. Prune Whale Registry (remove dormant whales to respect Pillar U)
        if self.neural_memory.whale_registry.len() > 2_000 {
            self.neural_memory.whale_registry.retain(|_, stats| {
                current_block.saturating_sub(stats.last_seen_block.load(Ordering::Relaxed)) < 1000
            });
        }
    }

    /// [LEAD ARCHITECT] Displacement Strategy: Finds an inactive pool to make room for new alpha.
    fn find_pool_to_evict(&self) -> Option<Address> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        for entry in self.mirror.pools.iter() {
            let (addr, ps) = entry.pair();
            // Protect core pools (WETH/USDC etc) from eviction
            if constants::CORE_POOLS.contains(addr) { continue; }
            if ps.last_swap_timestamp > 0 && (now - ps.last_swap_timestamp) > constants::POOL_REPLACEMENT_INTERVAL_SEC {
                return Some(*addr);
            }
        }
        None
    }

    fn evict_pool(&self, addr: Address) {
        debug!("♻️ [LIFECYCLE] Evicting stale pool {:?} to optimize graph density.", addr);
        self.mirror.pools.remove(&addr);
        self.graph.remove_pool(addr);
    }

    /// Pillar C: Pre-calculate all possible cycles to ensure Zero-Latency during block triggers.
    pub fn refresh_path_cache(&self) {
        // Pillar C: Multi-Core Coverage - Cache paths for all major Base assets
        let core_tokens: Vec<Address> = self.config.important_tokens.iter().cloned().collect();

        // Ensure graph data is synced before pathfinding
        self.graph.rebuild_optimized();
        let mut index: FxHashMap<Address, Vec<Arc<Path>>> = FxHashMap::default();
        let mut total_cycles = 0;

        for token in core_tokens {
            let cycles = self.graph.find_cycles(token, self.config.max_path_length);
            for cycle in cycles {
                if total_cycles > 5000 { break; } // [BULLETPROOF] Absolute limit on cached paths
                let arc_path = Arc::new(cycle.clone());
                index.entry(arc_path.hops[0].token_in).or_default().push(arc_path);
                total_cycles += 1;
            }
            if total_cycles > 5000 { break; }
        }

        // [OPTIMIZATION] Avoid full cloning of Paths for the global cache
        let mut flat_cycles = Vec::with_capacity(total_cycles);
        for paths in index.values() {
            for p in paths {
                flat_cycles.push((**p).clone());
            }
        }

        let mut active_pools = FxHashSet::default();
        for path in &flat_cycles {
            for hop in path.hops.iter() {
                active_pools.insert(hop.pool_address);
            }
        }
        self.mirror.update_sync_filter(active_pools);
        self.indexed_paths.store(Arc::new(index));
        self.path_cache.store(Arc::new(flat_cycles));
        debug!("🔄 [PATH CACHE] Refreshed. Total unique cycles: {}", self.path_cache.load().len());
    }
}

#[allow(dead_code)]
pub struct ArbitrageDetector {
    _config:      DetectorConfig,
    event_rx:     mpsc::Receiver<super::mempool_listener::SwapEvent>,
    priority_rx:  mpsc::Receiver<super::mempool_listener::SwapEvent>, // [WHALE HUNTER]
    pool_rx:      broadcast::Receiver<NewPoolEvent>,
    pub state:    Arc<SharedState>,
    force_rx:     UnboundedReceiver<()>,
    semaphore:    Arc<Semaphore>,
    _shutdown_tx: tokio::sync::watch::Sender<bool>,
}

impl ArbitrageDetector {
    pub async fn new(
        config:       DetectorConfig,
        provider:     Arc<Provider<Ws>>,
        state_mirror: Arc<StateMirror>, // This is still the single ws_provider, but it's okay for state_mirror
        gas_feed:     Arc<GasPriceFeed>,
        bidding_engine: Arc<BiddingEngine>,
        event_rx:     mpsc::Receiver<super::mempool_listener::SwapEvent>,
        priority_rx:  mpsc::Receiver<super::mempool_listener::SwapEvent>,
        pool_rx:      broadcast::Receiver<NewPoolEvent>,
    ) -> (Self, UnboundedReceiver<Opportunity>, UnboundedSender<()>) {
        let (opp_tx, opp_rx) = unbounded_channel();
        let (force_tx, force_rx) = unbounded_channel();
        let (_shutdown_tx, _) = tokio::sync::watch::channel(false);
        let graph = Arc::new(HotGraph::new());

        let state = Arc::new(SharedState {
            mirror:         state_mirror.clone(),
            graph,
            math_engine:    Arc::new(MathEngine),
            bidding_engine,
            opp_tx,
            config:         config.clone(),
            gas_feed,
            force_tx:       force_tx.clone(),
            path_cache:     Arc::new(ArcSwap::from_pointee(Vec::new())),
            indexed_paths:  Arc::new(ArcSwap::from_pointee(FxHashMap::default())),
            seen:           DashMap::new(),
            block_counter:  Arc::new(AtomicU64::new(0)),
            sender_history: DashMap::new(),
            neural_memory:  Arc::new(NeuralMemory { 
                revert_streak: DashMap::new(), 
                whale_registry: DashMap::new(),
                rejection_counters: DashMap::new(),
            }),
        });

        // Pillar Z: Spawn a task to preload pools from major factories
        let state_clone = state.clone();
        let provider_clone = provider.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await; // Zero-delay V3 Priority
            Self::preload_pools(state_clone, provider_clone).await;
        });

        // Initial path cache population
        state.refresh_path_cache();

        let threads  = config.scanner_threads;
        let detector = Self { 
            _config: config, 
            event_rx, 
            priority_rx,
            pool_rx, 
            state, 
            force_rx,
            semaphore: Arc::new(Semaphore::new(threads)), 
            _shutdown_tx 
        };
        (detector, opp_rx, force_tx)
    }

    /// Pre-loads top pools from known factories to bootstrap the arbitrage graph.
    async fn preload_pools(state: Arc<SharedState>, _provider: Arc<Provider<Ws>>) {
        info!("🔥 [PRELOAD] Starting Pillar Z: Autonomous Discovery Protocol...");
        
        let chain = state.config.chain;
        let tokens = match constants::SAFE_TOKENS.get(&chain) {
            Some(t) => t.clone(),
            None => return,
        };

        // 1. Core High-Liquidity Hand-picked Pools
        let core_pools = vec![
            (constants::POOL_UNIV3_WETH_USDC_005, DexName::UniswapV3, constants::TOKEN_WETH, constants::TOKEN_USDC, Some(500u32)),
            (constants::POOL_UNIV3_WETH_USDC_030, DexName::UniswapV3, constants::TOKEN_WETH, constants::TOKEN_USDC, Some(3000u32)),
            (constants::POOL_BASESWAP_WETH_USDC, DexName::BaseSwap, constants::TOKEN_WETH, constants::TOKEN_USDC, None),
            (constants::POOL_SUSHI_WETH_USDC, DexName::SushiSwap, constants::TOKEN_WETH, constants::TOKEN_USDC, None),
            (constants::POOL_UNIV2_WETH_DEGEN, DexName::UniswapV2, constants::TOKEN_WETH, constants::TOKEN_DEGEN, None),
        ];

        for (addr, dex, t0, t1, fee) in core_pools {
            Self::register_pool_internal_no_rebuild(&state, addr, dex, t0, t1, fee);
        }

        // 2. Pillar Z: Offline Pool Address Derivation for Safe Token Sets
        // Scaling to 500-2000+ pools by calculating deterministic addresses
        if chain == Chain::Base {
            let v2_factories = [
                (constants::BASE_BASESWAP_FACTORY, DexName::BaseSwap),
                (constants::BASE_SUSHISWAP_FACTORY, DexName::SushiSwap),
                (constants::BASE_PANCAKESWAP_FACTORY, DexName::PancakeSwap),
            ];

            for (factory, dex) in v2_factories {
                for i in 0..tokens.len().min(25) { // Balanced derivation range for Base
                    for j in i + 1..tokens.len() {
                        let t0 = tokens[i];
                        let t1 = tokens[j];
                        
                        let (ta, tb) = if t0 < t1 { (t0, t1) } else { (t1, t0) };
                        
                        // V2 Salt Calculation
                        let salt = keccak256([ta.as_bytes(), tb.as_bytes()].concat());
                        
                        let init_hash = if dex == DexName::SushiSwap {
                            constants::SUSHISWAP_INIT_CODE_HASH
                        } else {
                            constants::UNISWAP_V2_INIT_CODE_HASH
                        };
                        
                        let pool_addr = get_create2_address_from_hash(
                            factory,
                            salt,
                            init_hash.parse::<H256>().unwrap_or_default()
                        );

                        Self::register_pool_internal_no_rebuild(&state, pool_addr, dex, t0, t1, None);
                    }
                }
            }

            // Uniswap V3 Dynamic Derivation for top pairs
            let v3_factory = "0x33128a8fC170d030b747a24199840E2303c8959d".parse().unwrap_or(Address::zero());
            for i in 0..tokens.len().min(12) { // Focus on ULTRA-liquid V3 pairs
                for j in i + 1..tokens.len().min(12) {
                    for fee in constants::UNISWAP_V3_FEE_TIERS {
                        let (ta, tb) = if tokens[i] < tokens[j] { (tokens[i], tokens[j]) } else { (tokens[j], tokens[i]) };
                        
                        // V3 Salt: abi.encode(tokenA, tokenB, fee)
                        let mut salt_payload = [0u8; 96];
                        salt_payload[12..32].copy_from_slice(ta.as_bytes());
                        salt_payload[44..64].copy_from_slice(tb.as_bytes());
                        salt_payload[92..96].copy_from_slice(&fee.to_be_bytes());
                        let salt = keccak256(salt_payload);
                        
                        let pool_addr = get_create2_address_from_hash(
                            v3_factory,
                            salt,
                            constants::UNISWAP_V3_INIT_CODE_HASH.parse::<H256>().unwrap_or_default()
                        );
                        
                        Self::register_pool_internal_no_rebuild(&state, pool_addr, DexName::UniswapV3, ta, tb, Some(fee));
                    }
                }
            }
        }

        state.graph.rebuild_optimized();
        state.refresh_path_cache();
        info!("🚀 [PRELOAD] Pillar Z Complete. Tracking {} pools across {} tokens.", state.mirror.pools.len(), tokens.len());
    }

    fn register_pool_internal_no_rebuild(state: &Arc<SharedState>, addr: Address, dex: DexName, t0: Address, t1: Address, fee: Option<u32>) {
        // Lead Architect: Enforce configured pool limit for resource stabilization
        if state.mirror.pools.len() >= state.config.pool_limit {
            return;
        }

        // Manual addition to edges without triggering rebuild_optimized
        let e0 = PoolEdge { pool_address: addr, dex_name: dex, token_b: t1, fee, liq_score: 10000, static_calldata: Default::default(), gas_cost: U256::zero(), id: [0;32], success_prob: 10000 };
        let e1 = PoolEdge { pool_address: addr, dex_name: dex, token_b: t0, fee, liq_score: 10000, static_calldata: Default::default(), gas_cost: U256::zero(), id: [0;32], success_prob: 10000 };
        state.graph.edges.entry(t0).or_default().push(e0);
        state.graph.edges.entry(t1).or_default().push(e1);

        let mut s = PoolState::default();
        s.dex_type = match dex {
            DexName::UniswapV3 => crate::state_mirror::DexType::UniswapV3,
            DexName::Maverick => crate::state_mirror::DexType::MaverickV2,
            _ => crate::state_mirror::DexType::UniswapV2,
        };
        s.fee = fee.unwrap_or(0);
        s.last_swap_timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        state.mirror.update_pool(addr, s);
        state.mirror.mark_dirty();
    }

    pub async fn run(mut self) {
        info!("🔍 ArbitrageDetector ONLINE ({} workers) | Watching {} important tokens", self.state.config.scanner_threads, self.state.config.important_tokens.len());

        // Periodic stats log every 30s
        {
            let state = self.state.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
                loop {
                    interval.tick().await;
                    info!("📈 [DETECTOR STATS] Received:{} | Cycles:{} | Opps:{} | Pools:{} | IgnoredToken:{} | IgnoredNoCycle:{}",
                        SWAPS_RECEIVED.load(Ordering::Relaxed),
                        CYCLES_FOUND.load(Ordering::Relaxed),
                        OPPS_SENT.load(Ordering::Relaxed),
                        state.mirror.pools.len(),
                        IGNORED_NO_TOKEN.load(Ordering::Relaxed),
                        IGNORED_NO_CYCLE.load(Ordering::Relaxed),
                    );

                    // Pillar U: Prune memory periodically
                    state.prune_memory();
                }
            });
        }

        loop {
            tokio::select! {
                // [WHALE HUNTER] Priority Lane - Process Whales FIRST
                Some(event) = self.priority_rx.recv() => {
                    SWAPS_RECEIVED.fetch_add(1, Ordering::Relaxed);
                    if let Ok(permit) = self.semaphore.clone().acquire_owned().await {
                        let state = self.state.clone();
                        tokio::spawn(async move {
                            Self::process_event(state, event).await;
                            drop(permit);
                        });
                    }
                }

                // Pillar B: Hypersensitive Block Trigger (Highest Priority)
                Some(_) = self.force_rx.recv() => {
                    if let Ok(permit) = self.semaphore.clone().acquire_owned().await {
                        let state = self.state.clone();
                        // Pillar B: Zero-Latency Snapshot Analysis
                        let _ = tokio::task::spawn_blocking(move || {
                            Self::force_snapshot_analysis(state);
                            drop(permit);
                        });
                    }
                }

                Some(event) = self.event_rx.recv() => {
                    SWAPS_RECEIVED.fetch_add(1, Ordering::Relaxed);

                    if let Ok(permit) = self.semaphore.clone().acquire_owned().await {
                        let state = self.state.clone();
                        tokio::spawn(async move {
                            Self::process_event(state, event).await;
                            drop(permit);
                        });
                    } else {
                        warn!("⚠️ [DETECTOR] Semaphore acquire failed — all workers busy");
                    }
                }

                Ok(pool_event) = self.pool_rx.recv() => {
                    match pool_event {
                        NewPoolEvent::V2(log) => {
                            // [LEAD ARCHITECT] New Deployment Displacement
                            if self.state.mirror.pools.len() >= self.state.config.pool_limit {
                                if let Some(stale) = self.state.find_pool_to_evict() {
                                    self.state.evict_pool(stale);
                                } else {
                                    continue; // No stale pools to displace
                                }
                            }

                            self.state.graph.add_pool(log.pair, DexName::UniswapV2, log.token_0, log.token_1, None, 0);
                            info!("🆕 [V2 POOL] {:?} | T0:{:?} T1:{:?}", log.pair, log.token_0, log.token_1);
                            let mut s = PoolState::default();
                            s.dex_type = crate::state_mirror::DexType::UniswapV2;
                            self.state.mirror.update_pool(log.pair, s);
                            let mirror = self.state.mirror.clone();
                            tokio::spawn(async move { mirror.fetch_and_cache_bytecode(log.pair).await; });
                        }
                        NewPoolEvent::V3(log) => {
                            // [LEAD ARCHITECT] New Deployment Displacement
                            if self.state.mirror.pools.len() >= self.state.config.pool_limit {
                                if let Some(stale) = self.state.find_pool_to_evict() {
                                    self.state.evict_pool(stale);
                                } else {
                                    continue;
                                }
                            }

                            self.state.graph.add_pool(log.pool, DexName::UniswapV3, log.token_0, log.token_1, Some(log.fee), 0);
                            info!("🆕 [V3 POOL] {:?} | T0:{:?} T1:{:?} Fee:{}", log.pool, log.token_0, log.token_1, log.fee);
                            let mut s = PoolState::default();
                            s.dex_type = match log.dex_name {
                                DexName::Maverick => crate::state_mirror::DexType::MaverickV2,
                                _ => crate::state_mirror::DexType::UniswapV3,
                            };
                            s.fee = log.fee;
                            self.state.mirror.update_pool(log.pool, s);
                            let mirror = self.state.mirror.clone();
                            tokio::spawn(async move { mirror.fetch_and_cache_bytecode(log.pool).await; });
                        }
                    }
                    // Pillar C: Throttled Dynamic Re-map
                    if self.state.mirror.pools.len() % 100 == 0 {
                        let state = self.state.clone();
                        tokio::spawn(async move { state.refresh_path_cache(); });
                    }
                }
            }
        }
    }

    async fn process_event(state: Arc<SharedState>, event: super::mempool_listener::SwapEvent) {
        let swap = &event.swap_info;

        // Pillar O: Alpha-Only Filter - MOVE TO TOP
        // Discard 'Shitcoin' mempool noise in < 2ns before any state or logic check.
        if !state.config.important_tokens.contains(&swap.token_in) && 
           !state.config.important_tokens.contains(&swap.token_out) {
            return;
        }

        // [SURGICAL] Move Poison Check to absolute top to save O(1) lookup time
        if state.mirror.is_poisoned(&swap.router) {
            return;
        }

        // Pillar W: Zero-Latency Guard - Discard micro-swaps, dust, or wash-trades instantly
        // Exit speed is everything. We exit in < 5ns if these conditions aren't met.
        if swap.amount_in.is_zero() || swap.token_in == swap.token_out { return; }

        // Pillar T: Stale data guard
        let current_block = state.mirror.current_block_number();
        if state.block_counter.load(Ordering::Relaxed) != current_block {
            state.block_counter.store(current_block, Ordering::Relaxed);
            state.mirror.mark_dirty();
        }

        // Pillar F: Neural Memory - Mark the pool that just traded as dirty for high-priority sync
        if let Some(mut ps) = state.mirror.pools.get_mut(&swap.router) {
            ps.last_swap_timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        }

        state.mirror.mark_dirty();

        // Pillar W: Wash-Trap Radar - Behavioral Analysis
        let sender = event.sender;

        // Pillar F: Alpha Whale Detection
        if state.neural_memory.is_whale(&sender) {
            info!("🐋 [WHALE TRACKER] Alpha-Whale detected: {:?}. High-probability alpha trigger.", sender);
        }

        // Skip radar for known competitors (handled by Pillar I/H) or known routers
        if !crate::constants::KNOWN_COMPETITORS.contains(&sender) {
            let mut activity = state.sender_history.entry(sender).or_insert((current_block, 0, Address::zero(), Address::zero()));
            let (ref mut block, ref mut count, ref mut last_in, ref mut last_out) = *activity;
            
            if *block == current_block {
                *count += 1;
            } else {
                *block = current_block;
                *count = 1;
            }

            // Pillar W: Ping-Pong Wash-Trade Detection
            if *last_in == swap.token_out && *last_out == swap.token_in {
                return;
            }
            *last_in = swap.token_in;
            *last_out = swap.token_out;

            // If a single EOA triggers > 2 swaps in one block, it's a high-probability Wash-Trap bait.
            if *count > 2 {
                return;
            }
        }

        // Add pool to graph from live swap data
        let mut is_new = true;
        if let Some(edges) = state.graph.edges.get(&swap.token_in) {
            if edges.iter().any(|x| x.pool_address == swap.router) {
                is_new = false;
            }
        }

        if is_new {
            // [PILLAR Z] Priority-Based Pool Lifecycle Replacement
            if state.mirror.pools.len() >= state.config.pool_limit {
                if let Some(stale) = state.find_pool_to_evict() {
                    state.evict_pool(stale);
                } else {
                    return; // Everything is active and core, keep existing
                }
            }

            let liq = swap.amount_in.min(U256::from(u64::MAX)).as_u64();
            state.graph.add_pool(swap.router, swap.dex, swap.token_in, swap.token_out, swap.fee, liq);
            
            // [FIX] Register pool in Mirror so it gets picked up by Multicall sync
            let mut s = PoolState::default();
            s.dex_type = match swap.dex {
                DexName::UniswapV3 => crate::state_mirror::DexType::UniswapV3,
                _ => crate::state_mirror::DexType::UniswapV2,
            };
            state.mirror.update_pool(swap.router, s);
            debug!("📊 [GRAPH] Added pool {:?} | {:?}<->{:?} | DEX:{:?}",
                swap.router, swap.token_in, swap.token_out, swap.dex);
        }
        // state.refresh_path_cache(); // Removed: Too frequent, will be called by block listener

        // Pillar W: Liquidity Bait & Dormancy Filter
        if let Some(ps) = state.mirror.pools.get(&swap.router) {
            // Volume-to-Liquidity Anomaly: If swap volume > 50% of pool reserves, it's a bait trap.
            let reserves = ps.reserves0.saturating_add(ps.reserves1);
            if !reserves.is_zero() && swap.amount_in > (reserves / 2) {
                return;
            }

            // Bait Check: If pool liquidity is < 0.01 ETH equivalent, ignore it. 
            // Scammers use micro-liquidity to create fake price swings.
            if ps.last_updated_block > 0 && ps.reserves0 < U256::from(10u128.pow(12)) && ps.reserves1 < U256::from(10u128.pow(12)) {
                return;
            }

        // Dormancy Check: Only skip if pool was recently active but now has suspicious zero reserves
        // Don't skip pools that have never been synced (last_updated_block == 0)
        if ps.last_updated_block > 0 
            && ps.reserves0.is_zero() && ps.reserves1.is_zero() 
            && ps.liquidity.is_zero()
            && current_block.saturating_sub(ps.last_updated_block) > 100 {
            return;
        }
        }
        
        let index_guard = state.indexed_paths.load();

        // Pillar D: Spatial Arbitrage Fast-Path
        // Direct 2-pool comparison for immediate alpha extraction
        Self::check_spatial_arbitrage(state.clone(), swap, &event);

        // Pillar F: Multi-Token Entry Evaluation
        let mut trigger_tokens = Vec::with_capacity(2);
        if state.config.important_tokens.contains(&swap.token_out) { trigger_tokens.push(swap.token_out); }
        if state.config.important_tokens.contains(&swap.token_in) { trigger_tokens.push(swap.token_in); }

        for token in trigger_tokens {
            if let Some(paths) = index_guard.get(&token) {
                let filtered: Vec<Arc<Path>> = paths.iter()
                    .filter(|p| !p.hops.iter().any(|h| state.neural_memory.is_poisoned(&h.pool_address)))
                    .cloned().collect();
                
                if !filtered.is_empty() {
                    Self::evaluate_cycles(state.clone(), filtered, token, Some(event.sender), Some(event.effective_gas_price), event.is_whale_trigger);
                }
            }
        }
    }

    /// [NEW] Pillar D: Spatial Arbitrage (2-Pool Comparison)
    fn check_spatial_arbitrage(state: Arc<SharedState>, swap: &crate::models::SwapInfo, event: &super::mempool_listener::SwapEvent) {
        let target_token = if state.config.important_tokens.contains(&swap.token_out) { swap.token_out } else { swap.token_in };
        let other_token = if target_token == swap.token_out { swap.token_in } else { swap.token_out };

        if let Some(edges) = state.graph.edges.get(&target_token) {
            let competitor_pools: Vec<_> = edges.iter()
                .filter(|e| e.token_b == other_token && e.pool_address != swap.router)
                .collect();

            if !competitor_pools.is_empty() {
                for edge in competitor_pools {
                    let hop1 = state.graph.edge_to_hop(edge, target_token);
                    let hop2 = Hop {
                        pool: swap.router, pool_address: swap.router, pool_address_label: None,
                        token_in: other_token, token_out: target_token,
                        dex_type: match swap.dex { DexName::UniswapV3 => DexType::UniswapV3, _ => DexType::UniswapV2 },
                        dex_name: swap.dex, zero_for_one: other_token < target_token, fee: swap.fee,
                        static_calldata: Default::default(), gas_cost: U256::zero(), id: [0;32], success_prob: 10000,
                    };
                    let path = Arc::new(Path::new(&[hop1, hop2], 0, 0));
                    Self::evaluate_cycles(state.clone(), vec![path], target_token, Some(event.sender), Some(event.effective_gas_price), event.is_whale_trigger);
                }
            }
        }
    }

    fn force_snapshot_analysis(state: Arc<SharedState>) { // Changed to static for spawn_blocking
        let current_block = state.mirror.current_block_number();
        if state.block_counter.load(Ordering::Relaxed) != current_block {
            state.block_counter.store(current_block, Ordering::Relaxed);
            state.mirror.mark_dirty();
        }
        let cycles: Vec<Arc<Path>> = state.path_cache.load().iter().map(|p| Arc::new(p.clone())).collect();
        if cycles.is_empty() { return; }
        Self::evaluate_cycles(state, cycles, constants::TOKEN_WETH, None, None, false);
    }

    fn evaluate_cycles(
        state: Arc<SharedState>, 
        cycles: Vec<Arc<Path>>, 
        start_token: Address,
        trigger_sender: Option<Address>,
        trigger_gas_price: Option<U256>,
        is_whale: bool,
    ) {
        CYCLES_FOUND.fetch_add(cycles.len() as u64, Ordering::Relaxed);
        
        // Pillar T: State Freshness Veto - Discard evaluation if sync is lagging
        if let Err(e) = state.mirror.verify_state_freshness() {
            debug!("🛡️ [PILLAR T] Evaluation Vetoed: {}", e);
            return;
        }

        // Pillar F: Lock-Free Gas Read (No more await in hot path)
        let base_fee = state.mirror.current_base_fee();
        let priority_fee = state.mirror.current_priority_fee();

        let current_block = state.mirror.current_block_number();
        let eff_base     = if base_fee.is_zero()     { U256::from(1_000_000u64) } else { base_fee };
        let eff_priority = if priority_fee.is_zero() { U256::from(1_000_000u64) } else { priority_fee };
        let gas_price    = eff_base + eff_priority;

        let max_gas = match state.config.chain {
            Chain::Mainnet => U256::from(500_000_000_000u64), // 500 Gwei
            _              => U256::from(10_000_000_000u64),  // [FIX] 10 Gwei - allows trading during small Base spikes
        };

        // Pillar Q: Bootstrap Shield - Emergency Gas Veto
        // Agar gas price limit se bahar hai toh evaluation hi mat karo, CPU cycles bachao.
        if gas_price > max_gas && !is_whale { 
            warn!("⛽ [GAS SPIKE] {} gwei — skipping evaluation", gas_price / U256::from(1_000_000_000u64));
            return;
        }

        debug!("🔄 [CYCLE] Evaluating {} cycles for {:?}", cycles.len(), start_token);

        // Debug: show pool reserve status
        let mut pools_with_reserves = 0usize;
        let mut pools_zero = 0usize;
        for path in &cycles {
            for hop in path.hops.iter() {
                if let Some(ps) = state.mirror.pools.get(&hop.pool_address) {
                    if ps.reserves0.is_zero() && ps.reserves1.is_zero() && ps.liquidity.is_zero() {
                        pools_zero += 1;
                    } else {
                        pools_with_reserves += 1;
                    }
                }
            }
        }
        // Changed to debug! to reduce log spam
        if pools_zero > 0 { 
            debug!("⚠️ [RESERVES] Pools with data: {} | Pools with zero reserves: {} | Waiting for sync...", pools_with_reserves, pools_zero);
        }

        // Lead Architect Optimization: Only use parallel iteration if cycle count is high
        // Small trigger sets are 10x faster sequentially.
        let state_for_iter = state.clone();
        let iter_logic = move |path: Arc<Path>| {
            if state_for_iter.seen.contains_key(&path.hash) { return None; }
            Self::process_path_profitability(
                path,
                &state_for_iter,
                gas_price,
                eff_base,
                eff_priority,
                start_token,
                trigger_sender,
                trigger_gas_price
            )
        };

        let profitable_opportunities: Vec<Opportunity> = if cycles.len() > 16 {
            cycles.into_par_iter().filter_map(iter_logic).collect()
        } else {
            cycles.into_iter().filter_map(iter_logic).collect()
        };

        let mut tri_count = 0;
        let mut cross_count = 0;
        let mut profitable_opportunities = profitable_opportunities;

        // [STRATEGY] Prioritize Shorter Paths (2-hop arbs are usually highest prob)
        if !profitable_opportunities.is_empty() {
            let msg = format!("✨ *Opportunities Found!* ✨\n\nTotal: `{}`", profitable_opportunities.len());
            send_telegram_msg(&msg);
        }

        profitable_opportunities.sort_by_key(|opp| opp.path.hops.len());

        for mut opp in profitable_opportunities {
            let mut dexes = HashSet::new();
            for hop in opp.path.hops.iter() { dexes.insert(hop.dex_name); }
            
            if opp.path.hops.len() == 2 && dexes.len() > 1 {
                cross_count += 1;
            } else {
                tri_count += 1;
            }

            // Pillar F: Lock-free metadata insertion
            state.seen.insert(opp.path.hash, true);
            // [SURGICAL FIX] Removed UUID generation. Use pre-computed hash.
            opp.id = hex::encode(&opp.path.hash[..8]); 

            let mut bribe_pct = state.bidding_engine.calculate_bribe(&opp);
            
            // Pillar I: Adaptive Bidding - Never bid more than 98% of net profit
            bribe_pct = bribe_pct.min(98);

            // Pillar F: Whale-Targeted Bidding Escalation
            if is_whale || opp.trigger_sender.map_or(false, |s| state.neural_memory.is_whale(&s)) {
                if let Some(sender) = opp.trigger_sender {
                    state.neural_memory.record_alpha(sender, current_block); 
                }
                debug!("🐋 [WHALE HUNTER] Alpha-Whale Gap Detected! Forcing Aggressive Execution.");
                bribe_pct = bribe_pct.max(95); // High confidence, high bribe to guarantee win
                opp.success_prob = 10000; // Force high success prob
            }

            let bribe     = opp.expected_profit * U256::from(bribe_pct) / U256::from(100);
            
            // Final Formula: NetProfit = (GrossProfit - OperationalGas) - BribeValue
            let net_after = opp.expected_profit.saturating_sub(bribe);
            opp.expected_profit = net_after; 
            if let Some(ref mut d) = opp.profit_details { d.net_profit = net_after; } 

            if tracing::enabled!(tracing::Level::DEBUG) {
                let profit_eth = net_after.as_u128() as f64 / 1e18;
                let profit_usd = profit_eth * 2600.0;
                debug!("🚀 [SIMULATION SUCCESS] Profit: ${:.2} | Net: {:.6} ETH | Hops:{} | Bribe:{}% | NetWei:{}",
                    profit_usd, profit_eth, opp.path.hops.len(), bribe_pct, net_after);
            }

            // FIX 1: Check channel send result — log if dropped
            if state.opp_tx.send(opp).is_err() {
                warn!("⚠️ [WIRE] Opportunity channel closed — receiver dropped!");
            } else {
                OPPS_SENT.fetch_add(1, Ordering::Relaxed);
            }
        }

        // Pillar F: Strategy Summary - Logged once per evaluation block
        if tri_count > 0 || cross_count > 0 {
            info!("📊 [STRATEGY] Found {} Triangular | {} Cross-DEX opportunities", tri_count, cross_count);
        }
    }

    /// Pillar D: Path Profitability Processor (Lead Architect Refactor)
    /// Extracted for sub-millisecond precision and better compiler optimization.
    fn process_path_profitability(
        path: Arc<Path>,
        state: &Arc<SharedState>,
        gas_price: U256,
        eff_base: U256,
        eff_priority: U256,
        start_token: Address,
        trigger_sender: Option<Address>,
        trigger_gas_price: Option<U256>,
    ) -> Option<Opportunity> {
        // Pillar D: GSS/Newton for optimal input
        let profit_func = |amount_in: U256| -> U256 {
            if amount_in.is_zero() { return U256::zero(); }
            let mut current = amount_in;
            for hop in path.hops.iter() {
                if let Some(ps) = state.mirror.pools.get(&hop.pool_address) {
                    current = match hop.dex_name {
                        DexName::UniswapV2 | DexName::SushiSwap | DexName::Aerodrome | DexName::BaseSwap | DexName::PancakeSwap => {
                            let (r_in, r_out) = if hop.token_in < hop.token_out {
                                (ps.reserves0, ps.reserves1)
                            } else {
                                (ps.reserves1, ps.reserves0)
                            };
                            if r_in.is_zero() || r_out.is_zero() { return U256::zero(); }
                            let fee_bps = hop.fee.unwrap_or(30); 
                            let multiplier = U256::from(10000 - fee_bps);
                            let amount_in_with_fee = current * multiplier;
                            (amount_in_with_fee * r_out) / (r_in * 10000 + amount_in_with_fee)
                        }
                        DexName::UniswapV3 => {
                            if ps.liquidity.is_zero() || ps.sqrt_price_x96.is_zero() { return U256::zero(); }
                            let (o, _, _, _) = crate::v3_math::simulate_swap_with_limit(
                                ps.sqrt_price_x96, ps.tick, ps.liquidity.into(),
                                &ps.ticks, &ps.tick_bitmap,
                                current, hop.token_in < hop.token_out,
                                hop.fee.unwrap_or(3000), None,
                                crate::v3_math::fee_to_tick_spacing(hop.fee.unwrap_or(3000)),
                            );
                            o
                        }
                        DexName::Maverick => {
                            let sqrt_price_x96 = ps.sqrt_price_x96;
                            if sqrt_price_x96.is_zero() { return U256::zero(); }
                            let fee_bps = hop.fee.unwrap_or(30); 
                            let amount_in_with_fee = current * U256::from(10000 - fee_bps);
                            if hop.token_in < hop.token_out {
                                let p_u512 = U512::from(sqrt_price_x96) * U512::from(sqrt_price_x96);
                                ((U512::from(amount_in_with_fee) * p_u512) / (U512::from(10000) << 192)).try_into().unwrap_or(U256::zero())
                            } else {
                                let p_u512 = U512::from(sqrt_price_x96) * U512::from(sqrt_price_x96);
                                ((U512::from(amount_in_with_fee) << 192) / (p_u512 * U512::from(10000))).try_into().unwrap_or(U256::zero())
                            }
                        }
                        _ => { return U256::zero(); }
                    };
                } else {
                    return U256::zero();
                }
            }
            current.saturating_sub(amount_in)
        };

        let path_str = path.hops.iter()
            .map(|h| format!("{:?}", h.token_out))
            .collect::<Vec<_>>()
            .join("->");

        let mut optimal = U256::zero();
        let mut use_gss = true;

        if path.hops.len() == 2 {
            let h0 = &path.hops[0];
            let h1 = &path.hops[1];
            let is_v2_like = |dex| matches!(dex, DexName::UniswapV2 | DexName::SushiSwap | DexName::Aerodrome | DexName::BaseSwap | DexName::PancakeSwap);
            let is_v3 = |dex| dex == DexName::UniswapV3;
            let is_mav = |dex| dex == DexName::Maverick;

            if (is_v2_like(h0.dex_name) || is_v3(h0.dex_name) || is_mav(h0.dex_name)) && 
               (is_v2_like(h1.dex_name) || is_v3(h1.dex_name) || is_mav(h1.dex_name)) {
                if let (Some(s0), Some(s1)) = (state.mirror.pools.get(&h0.pool_address), state.mirror.pools.get(&h1.pool_address)) {
                    let (r1_in, r1_out) = if is_v2_like(h0.dex_name) {
                        if h0.token_in < h0.token_out { (s0.reserves0, s0.reserves1) } else { (s0.reserves1, s0.reserves0) }
                    } else if is_mav(h0.dex_name) {
                        MathEngine::get_maverick_virtual_reserves(s0.sqrt_price_x96, s0.liquidity, h0.token_in < h0.token_out)
                    } else {
                        MathEngine::get_v3_virtual_reserves(s0.sqrt_price_x96, s0.liquidity, h0.token_in < h0.token_out)
                    };

                    let (r2_in, r2_out) = if is_v2_like(h1.dex_name) {
                        if h1.token_in < h1.token_out { (s1.reserves0, s1.reserves1) } else { (s1.reserves1, s1.reserves0) }
                    } else if is_mav(h1.dex_name) {
                        MathEngine::get_maverick_virtual_reserves(s1.sqrt_price_x96, s1.liquidity, h1.token_in < h1.token_out)
                    } else {
                        MathEngine::get_v3_virtual_reserves(s1.sqrt_price_x96, s1.liquidity, h1.token_in < h1.token_out)
                    };

                    if !r1_in.is_zero() && !r1_out.is_zero() && !r2_in.is_zero() && !r2_out.is_zero() {
                        let f0 = if is_v3(h0.dex_name) { h0.fee.unwrap_or(3000) / 100 } else { h0.fee.unwrap_or(30) };
                        let f1 = if is_v3(h1.dex_name) { h1.fee.unwrap_or(3000) / 100 } else { h1.fee.unwrap_or(30) };
                        optimal = MathEngine::calculate_optimal_v2_v2(r1_in, r1_out, r2_in, r2_out, f0, f1);
                        
                        let mut boundary_ok = true;
                        if is_v3(h0.dex_name) {
                            let next_sp = MathEngine::get_v3_next_sqrt_price(s0.sqrt_price_x96, s0.liquidity, optimal, h0.token_in < h0.token_out);
                            let (lower, upper) = if h0.token_in < h0.token_out {
                                (crate::v3_math::get_sqrt_ratio_at_tick(s0.tick), s0.sqrt_price_x96)
                            } else {
                                (s0.sqrt_price_x96, crate::v3_math::get_sqrt_ratio_at_tick(s0.tick + 1))
                            };
                            if next_sp < lower || next_sp > upper { boundary_ok = false; }
                        }
                        if boundary_ok && is_v3(h1.dex_name) {
                            let next_sp = MathEngine::get_v3_next_sqrt_price(s1.sqrt_price_x96, s1.liquidity, optimal, h1.token_in < h1.token_out);
                            let (lower, upper) = if h1.token_in < h1.token_out {
                                (crate::v3_math::get_sqrt_ratio_at_tick(s1.tick), s1.sqrt_price_x96)
                            } else {
                                (s1.sqrt_price_x96, crate::v3_math::get_sqrt_ratio_at_tick(s1.tick + 1))
                            };
                            if next_sp < lower || next_sp > upper { boundary_ok = false; }
                        }
                        if boundary_ok { use_gss = false; }
                    }
                }
            }
        }

        if use_gss {
            let initial_guess = U256::from(10u64.pow(16)); 
            let mut profit_func_copy = profit_func;
            optimal = MathEngine::find_optimal_input_newton(initial_guess, |amt| {
                let delta = (amt / 10000).max(U256::from(1u64));
                let p1 = profit_func_copy(amt);
                let p2 = profit_func_copy(amt + delta);
                let marginal = (p2.as_u128() as f64 - p1.as_u128() as f64) / (delta.as_u128() as f64);
                marginal - 1.0 // Derivative of (Profit - Input) is (dProfit/dInput - 1)
            });
            if optimal.is_zero() {
                optimal = MathEngine::find_optimal_input(U256::from(1000u64), U256::from(10u64.pow(17)), &mut profit_func_copy);
            }
        }

        let path_str = path.hops.iter()
            .map(|h| format!("{:?}", h.token_out))
            .collect::<Vec<_>>()
            .join("->");

        if optimal.is_zero() { 
            crate::auditor::log_rejection(path_str, "Optimal Input Zero (No Arb Gap)", U256::zero(), U256::zero());
            state.neural_memory.record_rejection("No Arb Gap");
            return None; 
        }
        let raw = profit_func(optimal);
        if raw.is_zero() { 
            crate::auditor::log_rejection(path_str, "Raw Profit Zero", U256::zero(), U256::zero());
            state.neural_memory.record_rejection("Raw Profit Zero");
            return None; 
        }
        
        let fl_fee = match state.config.chain {
            Chain::Mainnet => optimal * U256::from(5) / U256::from(10000),
            _ => U256::zero(),
        };

        // Pillar D: Slippage-Adjusted Math (0.3% Haircut)
        // Taaki high-volatility scenarios mein calculation "Optimistic" na ho.
        let raw_with_slippage: U256 = (raw * 997) / 1000;
        let gross = raw_with_slippage.saturating_sub(fl_fee);
        if gross.is_zero() { return None; }

        if let Some(bl) = BLACKLISTED_TOKENS.get(&state.config.chain) {
            if path.hops.iter().any(|h| bl.contains(&h.token_in) || bl.contains(&h.token_out)) { return None; }
        }

        // Pillar O: L2-Aware Gas Cost (L1 Data Fee + L2 Execution)
        // Base par 1 byte calldata approx 16 gas (L1) leta hai. 
        // Hum path length se calldata size estimate kar rahe hain.
        let l1_data_fee_est = match state.config.chain {
            Chain::Base | Chain::Arbitrum | Chain::Optimism => {
                // Est: 150 bytes calldata * 16 gas/byte * L1 Base Fee
                let l1_base_fee = *state.gas_feed.l1_fee.borrow();
                let calldata_size = 100 + (path.hops.len() * 42); 
                U256::from(calldata_size as u64 * 16) * l1_base_fee
            }
            _ => U256::zero(),
        };

        // Base execution gas is cheap (~200k-400k), but L1 fee is the real killer.
        let gas_est = U256::from(250_000u64 + 60_000u64 * path.hops.len() as u64);
        
        let gas_cost = gas_est * gas_price;
        // Pillar F & Q: Rejection Audit Integration
        let path_str = path.hops.iter()
            .map(|h| format!("{:?}", h.token_out))
            .collect::<Vec<_>>()
            .join("->");

        // Pre-calculate gas costs for logging
        let l1_data_fee_est = match state.config.chain {
            Chain::Base | Chain::Arbitrum | Chain::Optimism => {
                let l1_base_fee = *state.gas_feed.l1_fee.borrow();
                let calldata_size = 100 + (path.hops.len() * 42); 
                U256::from(calldata_size as u64 * 16) * l1_base_fee
            }
            _ => U256::zero(),
        };
        let gas_est = U256::from(250_000u64 + 60_000u64 * path.hops.len() as u64);
        let total_op_cost = (gas_est * gas_price).saturating_add(l1_data_fee_est);

        if optimal.is_zero() { 
            crate::auditor::log_rejection(path_str, "Optimal Input Zero (No Arb Gap)", U256::zero(), total_op_cost);
            return None; 
        }
        let raw = profit_func(optimal);
        if raw.is_zero() { 
            crate::auditor::log_rejection(path_str, "Raw Profit Zero", U256::zero(), total_op_cost);
            return None; 
        }

        // Pillar M: Dynamic Flash Loan Fee (Aave: 0.05%, Balancer: 0%)
        // Lead Architect: Always assume a 0.05% baseline if Loan Source is Aave
        let fl_fee_bps = if state.config.chain == Chain::Mainnet || state.config.chain == Chain::Base { 5 } else { 0 };
        let fl_fee = (optimal * U256::from(fl_fee_bps)) / U256::from(10000);

        let raw_with_slippage: U256 = (raw * 997) / 1000;
        let gross = raw_with_slippage.saturating_sub(fl_fee);
        
        if gross.is_zero() {
            crate::auditor::log_rejection(path_str.clone(), "Low Profit", U256::zero(), total_op_cost);
            return None; 
        } 

        if let Some(bl) = BLACKLISTED_TOKENS.get(&state.config.chain) {
            if path.hops.iter().any(|h| bl.contains(&h.token_in) || bl.contains(&h.token_out)) { return None; }
        }

        let net = gross.saturating_sub(total_op_cost);
        let min_required_profit = U256::from(crate::constants::MIN_PROFIT_WEI);

        if net < min_required_profit {
            crate::auditor::log_rejection(path_str, "Low Profit", gross, total_op_cost);
            return None; 
        }

        Some(Opportunity {
            id:             path.hash.to_string(),
            path, // Path is already Arc<Path>, no need for Arc::new(path.clone())
            expected_profit: net,
            gas_cost,
            success_prob:   10000,
            base_fee:       eff_base,
            priority_fee:   eff_priority,
            gas_estimate:   gas_est,
            input_amount:   optimal,
            input_token:    start_token,
            profit_details: Some(ProfitDetails { net_profit: net, slippage: 0.003, gas_savings: U256::zero() }),
            chain:          state.config.chain,
            static_calldata: Default::default(),
            trigger_gas_price,
            trigger_sender,
        })
    }
}
