use alloy_primitives::{Address, U256};
use alloy::providers::RootProvider;
use alloy::transports::BoxTransport;
use alloy::signers::local::PrivateKeySigner as LocalWallet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Semaphore, mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender}};
use tracing::{info, debug};
use rustc_hash::FxHasher;
use std::hash::BuildHasherDefault;
use arc_swap::ArcSwap;
use tokio::time::{self, Duration};
use dashmap::DashMap;
use rustc_hash::{FxHashMap, FxHashSet};
use crate::gas_feed::GasPriceFeed;
use crate::bidding_engine::BiddingEngine;
use crate::models::{Chain, Opportunity, DexName, Path, Hop, DexType, PoolEdge, MempoolTx};
use crate::state_mirror::StateMirror;
use crate::constants;
use crate::factory_scanner::NewPoolEvent;
use crate::math_engine::MathEngine;

pub static SWAPS_RECEIVED:   AtomicU64   = AtomicU64::new(0);
pub static CYCLES_FOUND:     AtomicU64   = AtomicU64::new(0);
pub static OPPS_SENT:        AtomicU64   = AtomicU64::new(0);
pub static IGNORED_NO_TOKEN: AtomicU64   = AtomicU64::new(0);
pub static IGNORED_NO_CYCLE: AtomicU64   = AtomicU64::new(0);
pub static ACTIVE_POOLS_COUNT: AtomicU64 = AtomicU64::new(0);

#[derive(Default, Clone)]
pub struct StaticGraph {
    pub tokens: Vec<Address>,
    pub token_to_idx: FxHashMap<Address, u32>,
    pub pools: Vec<Address>,
    pub pool_to_idx: FxHashMap<Address, u32>,
    pub nodes: Vec<u32>, 
    pub edges: Vec<StaticEdge>,
}

#[derive(Clone)]
pub struct StaticEdge {
    pub pool_idx: u32,
    pub target_token_idx: u32,
    pub dex_name: DexName,
}

#[derive(Clone)]
pub struct DetectorConfig {
    pub min_profit_wei: U256,
    pub max_path_length: usize,
    pub important_tokens: Arc<HashSet<Address>>,
    pub multicall_address: Address,
    pub factories: HashMap<DexName, Address>,
    pub scanner_threads: usize,
    pub min_liquidity_eth: u64,
    pub chain: Chain,
    pub priority_fee_percent: u64,
    pub bribe_percent: u64,
    pub flashbots_relay: Option<String>,
    pub signer: Option<LocalWallet>,
    pub executor_address: Address,
    pub pool_limit: usize,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        let mut important = HashSet::new();
        if let Some(tokens) = constants::SAFE_TOKENS.get(&Chain::Base) {
            for token in tokens { important.insert(*token); }
        }
        Self {
            min_profit_wei: U256::from(1u64),
            max_path_length: constants::MAX_HOPS,
            important_tokens: Arc::new(important),
            multicall_address: "0xcA11bde05977b3631167028862bE2a173976CA11".parse().unwrap_or(Address::ZERO),
            factories: HashMap::new(),
            scanner_threads: 8,
            min_liquidity_eth: 1,
            chain: Chain::Base,
            priority_fee_percent: 10,
            bribe_percent: 50,
            flashbots_relay: None,
            signer: None,
            executor_address: Address::ZERO,
            pool_limit: 3500,
        }
    }
}

pub struct ArbitrageDetector {
    config: DetectorConfig,
    event_rx: mpsc::Receiver<super::mempool_listener::SwapEvent>,
    priority_rx: mpsc::Receiver<super::mempool_listener::SwapEvent>,
    pool_rx: broadcast::Receiver<NewPoolEvent>,
    state_mirror: Arc<StateMirror>,
    _gas_feed: Arc<GasPriceFeed>,
    opp_tx: UnboundedSender<Opportunity>,
    force_rx: UnboundedReceiver<()>,
    semaphore: Arc<Semaphore>,
    graph: Arc<ArcSwap<StaticGraph>>,
    pool_registry: Arc<DashMap<Address, (Address, Address, DexName), BuildHasherDefault<FxHasher>>>,
    pool_activity: Arc<DashMap<Address, std::time::Instant>>, // Tracks "Heat"
    alpha_grace_period: Arc<DashMap<Address, std::time::Instant>>, // New Launches
}

impl ArbitrageDetector {
    pub async fn new(
        config: DetectorConfig,
        _provider: Arc<RootProvider<BoxTransport>>,
        state_mirror: Arc<StateMirror>,
        gas_feed: Arc<GasPriceFeed>,
        _bidding_engine: Arc<BiddingEngine>,
        event_rx: mpsc::Receiver<super::mempool_listener::SwapEvent>,
        priority_rx: mpsc::Receiver<super::mempool_listener::SwapEvent>,
        pool_rx: broadcast::Receiver<NewPoolEvent>,
    ) -> (Self, UnboundedReceiver<Opportunity>, UnboundedSender<()>) {
        let (opp_tx, opp_rx) = unbounded_channel();
        let (force_tx, force_rx) = unbounded_channel();
        let threads = config.scanner_threads;
        let detector = Self {
            config,
            event_rx,
            priority_rx,
            pool_rx,
            state_mirror,
            _gas_feed: gas_feed,
            opp_tx,
            force_rx,
            semaphore: Arc::new(Semaphore::new(threads)),
            graph: Arc::new(ArcSwap::from_pointee(StaticGraph::default())),
            pool_registry: Arc::new(DashMap::with_hasher(BuildHasherDefault::default())),
            pool_activity: Arc::new(DashMap::new()),
            alpha_grace_period: Arc::new(DashMap::new()),
        };
        (detector, opp_rx, force_tx)
    }

    pub async fn run(mut self) {
        info!("🔍 [PILLAR C] Pathfinding Engine ACTIVE — Hunting complex cycles up to {} hops", constants::MAX_HOPS);
        let math = MathEngine;

        // Initial graph build
        self.rebuild_graph();

        let mut sync_interval = time::interval(Duration::from_secs(60)); // Faster registry sync

        loop {
            tokio::select! {
                _ = sync_interval.tick() => { self.sync_registry(); }
                Some(event) = self.priority_rx.recv() => {
                    SWAPS_RECEIVED.fetch_add(1, Ordering::Relaxed);
                    self.process_event(event, math).await;
                }
                Some(event) = self.event_rx.recv() => {
                    SWAPS_RECEIVED.fetch_add(1, Ordering::Relaxed);
                    // Pillar Z: Update pool activity to keep graph "Hot"
                    if event.swap_info.token_in.is_zero() {
                        // Event-based trigger (Back-running)
                        self.pool_activity.insert(event.swap_info.router, std::time::Instant::now());
                    } else {
                        if let Some(pool) = self.get_pool_from_router(&event.swap_info.router, &event.swap_info.token_in, &event.swap_info.token_out) {
                            self.pool_activity.insert(pool, std::time::Instant::now());
                        }
                    }
                    self.process_event(event, math).await;
                }
                Some(_) = self.force_rx.recv() => {
                    self.rebuild_graph();
                }
                Ok(pool_event) = self.pool_rx.recv() => {
                    // POWER TECH 1: Reactive Injection
                    // Naye pool ko turant registry mein daalo aur graph rebuild trigger karo.
                    match pool_event {
                        NewPoolEvent::V2(data) => { 
                            self.pool_registry.insert(data.pair, (data.token_0, data.token_1, data.dex_name));
                            self.alpha_grace_period.insert(data.pair, std::time::Instant::now());
                        }
                        NewPoolEvent::V3(data) => { 
                            self.pool_registry.insert(data.pool, (data.token_0, data.token_1, data.dex_name));
                            self.alpha_grace_period.insert(data.pool, std::time::Instant::now());
                        }
                    }
                    self.rebuild_graph(); // Trigger immediate graph refresh for the new Alpha
                }
            }
        }
    }

    async fn process_event(&self, event: crate::mempool_listener::SwapEvent, math: MathEngine) {
        let state_mirror = self.state_mirror.clone();
        let opp_tx = self.opp_tx.clone();
        let graph = self.graph.load().clone();
        let config = self.config.clone();

        // Pillar T: Sub-block State Projection
        let impacts = if let Some(ref mempool_tx) = event.mempool_tx {
            if event.swap_info.amount_in < U256::from(2 * 10u128.pow(16)) { return; } // < 0.02 ETH
            math.project_reserve_impact(mempool_tx, &state_mirror)
        } else {
            FxHashMap::default()
        };

        // Pillar C: Multi-Directional Back-running Search (In-Memory Math)
        let mut search_directions = Vec::new();

        if event.swap_info.token_in.is_zero() {
            // Back-running trigger: Evaluate both directions of the pool using RAM state
            if let Some(reg) = self.pool_registry.get(&event.swap_info.router) {
                search_directions.push((reg.0, reg.1));
                search_directions.push((reg.1, reg.0));
            }
        } else {
            // Front-running trigger: Evaluate specific swap direction
            search_directions.push((event.swap_info.token_in, event.swap_info.token_out));
        }

        for (t_in, t_out) in search_directions {
            let start_token_idx = match graph.token_to_idx.get(&t_in) {
                Some(idx) => *idx,
                None => continue,
            };
            let current_token_idx = match graph.token_to_idx.get(&t_out) {
                Some(idx) => *idx,
                None => continue,
            };

            let start_idx = graph.nodes[current_token_idx as usize] as usize;
            let end_idx = graph.nodes[current_token_idx as usize + 1] as usize;

            for i in start_idx..end_idx {
            let edge = graph.edges[i].clone();
            let graph_c = graph.clone();
            let state_mirror_c = state_mirror.clone();
            let opp_tx_c = opp_tx.clone();
            let config_c = config.clone();
            let impacts_c = impacts.clone();
            let event_c = event.clone();
            let semaphore_c = self.semaphore.clone();

            tokio::spawn(async move {
                let _permit = match semaphore_c.acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => return,
                };

                let mut paths = Vec::new();
                let mut current_hops = Vec::new();
                let mut visited_pools = [0u64; 64];
                
                if let Some(trigger_idx) = graph_c.pool_to_idx.get(&event_c.swap_info.router) {
                    visited_pools[*trigger_idx as usize >> 6] |= 1 << (*trigger_idx & 63);
                }

                // Process first hop manually for this task branch
                let pool_addr = graph_c.pools[edge.pool_idx as usize];
                let token_in = graph_c.tokens[current_token_idx as usize];
                let token_out = graph_c.tokens[edge.target_token_idx as usize];
                let is_stable = state_mirror_c.pools.get(&pool_addr).map(|p| p.is_stable).unwrap_or(false);

                let hop = Hop {
                    pool: pool_addr, pool_address: pool_addr,
                    token_in, token_out,
                    is_stable,
                    dex_type: match edge.dex_name { 
                        DexName::Aerodrome => DexType::Aerodrome,
                        DexName::UniswapV3 => DexType::UniswapV3,
                        _ => DexType::UniswapV2
                    },
                    dex_name: edge.dex_name,
                    zero_for_one: token_in < token_out,
                    ..Default::default()
                };

                current_hops.push(hop);
                visited_pools[edge.pool_idx as usize >> 6] |= 1 << (edge.pool_idx & 63);

                if edge.target_token_idx == start_token_idx {
                    paths.push(current_hops.clone());
                } else {
                    Self::find_cycles_dfs(
                        edge.target_token_idx,
                        start_token_idx,
                        &graph_c,
                        &state_mirror_c,
                        &mut current_hops,
                        &mut visited_pools,
                        &mut paths,
                        2, // Depth 2 because first hop is already done
                        config_c.max_path_length,
                    );
                }

                let math = MathEngine;
                for hops in paths {
                    let mut fetched_states = Vec::with_capacity(hops.len());
                    for hop in &hops {
                        if let Some(mut state) = state_mirror_c.get_pool_data(&hop.pool_address, 10) {
                            if let Some((r0, r1)) = impacts_c.get(&hop.pool_address) {
                                state.reserves0 = *r0;
                                state.reserves1 = *r1;
                            }
                            fetched_states.push(state);
                        } else {
                            break;
                        }
                    }
                    if fetched_states.len() != hops.len() { continue; }
                    
                    // RAM MATH: All path calculations happen here in microseconds
                    // No RPC calls are made during optimal input discovery
                    let path = Arc::new(Path::new(&hops, 200_000));
                    let start_token = hops[0].token_in;
                    let contains_aerodrome = hops.iter().any(|h| h.dex_name == DexName::Aerodrome);

                    let (optimal_in, profit) = if contains_aerodrome {
                        let initial_guess = U256::from(10u128.pow(17));
                        let opt_in = MathEngine::find_optimal_input_newton(initial_guess, |amt: U256| {
                            if amt.is_zero() { return 0.0; }
                            let mut path_marginal_price = 1.0;
                            for (i, hop) in path.hops.iter().enumerate() {
                                    let state = &fetched_states[i];
                                    let hop_price = if hop.dex_name == DexName::Aerodrome && state.is_stable {
                                        let (r_in, r_out) = if hop.zero_for_one { (state.reserves0, state.reserves1) } else { (state.reserves1, state.reserves0) };
                                        math.get_aerodrome_marginal_price(r_in, r_out)
                                    } else {
                                        let delta = U256::from(10u128.pow(14));
                                        let out_start = math.get_path_output(&Path::new(&[hop.clone()], 0), amt, &state_mirror_c);
                                        let out_end = math.get_path_output(&Path::new(&[hop.clone()], 0), amt + delta, &state_mirror_c);
                                        U256::from(((out_end.saturating_sub(out_start)).to::<u128>() as f64 / 1e14 * 1e18) as u128)
                                    };
                                    path_marginal_price *= hop_price.to::<u128>() as f64 / 1e18;
                            }
                            path_marginal_price
                        });
                        let out = math.get_path_output_with_states(&path.hops, opt_in, &fetched_states);
                        (opt_in, out.saturating_sub(opt_in))
                    } else {
                        MathEngine::find_optimal_input(U256::from(10u128.pow(15)), U256::from(100u128 * 10u128.pow(18)), |amt| {
                            let out = math.get_path_output_with_states(&path.hops, amt, &fetched_states);
                            alloy_primitives::I256::try_from(out.saturating_sub(amt)).unwrap_or_default()
                        })
                    };

                    if profit > config_c.min_profit_wei {
                        CYCLES_FOUND.fetch_add(1, Ordering::Relaxed);
                        let opp = Opportunity {
                            id: format!("cycle-{}", CYCLES_FOUND.load(Ordering::Relaxed)),
                            path, expected_profit: profit, input_token: start_token, input_amount: optimal_in,
                            pending_txs: event_c.mempool_tx.as_ref().map(|m: &MempoolTx| vec![m.clone()]).unwrap_or_default(),
                            chain: config_c.chain, trigger_sender: Some(event_c.sender), ..Default::default()
                        };
                        let _ = opp_tx_c.send(opp);
                    }
                }
            });
            }
        }
    }

    fn find_cycles_dfs(
        current_token_idx: u32,
        target_token_idx: u32,
        graph: &StaticGraph,
        state_mirror: &StateMirror,
        current_hops: &mut Vec<Hop>,
        visited_pools: &mut [u64; 64],
        results: &mut Vec<Vec<Hop>>,
        depth: usize,
        max_depth: usize,
    ) {
        if depth > max_depth { return; }

        let start = graph.nodes[current_token_idx as usize] as usize;
        let end = graph.nodes[current_token_idx as usize + 1] as usize;

        for i in start..end {
            let edge = &graph.edges[i];
            if (visited_pools[edge.pool_idx as usize >> 6] >> (edge.pool_idx & 63)) & 1 == 1 { continue; }

            let pool_addr = graph.pools[edge.pool_idx as usize];
            let token_in = graph.tokens[current_token_idx as usize];
            let token_out = graph.tokens[edge.target_token_idx as usize];
            let is_stable = state_mirror.pools.get(&pool_addr).map(|p| p.is_stable).unwrap_or(false);

            let hop = Hop {
                pool: pool_addr, pool_address: pool_addr,
                token_in, token_out,
                is_stable,
                dex_type: match edge.dex_name { 
                    DexName::Aerodrome => DexType::Aerodrome,
                    DexName::UniswapV3 => DexType::UniswapV3,
                    _ => DexType::UniswapV2
                },
                dex_name: edge.dex_name,
                zero_for_one: token_in < token_out,
                ..Default::default()
            };

            current_hops.push(hop);
            visited_pools[edge.pool_idx as usize >> 6] |= 1 << (edge.pool_idx & 63);

            // Atomic Multi-Hop Logic:
            // Agar hum target token (e.g. ETH) tak pahunch gaye hain aur hops 2 ya usse zyada hain (Total cycle 3+),
            // toh ise ek valid multi-hop path maana jayega.
            if edge.target_token_idx == target_token_idx && current_hops.len() >= 1 {
                results.push(current_hops.clone());
            } else {
                Self::find_cycles_dfs(edge.target_token_idx, target_token_idx, graph, state_mirror, current_hops, visited_pools, results, depth + 1, max_depth);
            }

            visited_pools[edge.pool_idx as usize >> 6] &= !(1 << (edge.pool_idx & 63));
            current_hops.pop();
        }
    }

    /// Pillar S: Memory Leak Prevention.
    /// Syncs the registry with StateMirror to remove stale/dead pools from memory.
    pub fn sync_registry(&self) {
        let mirror_pools = &self.state_mirror.pools;
        self.pool_registry.retain(|addr, _| {
            mirror_pools.contains_key(addr)
        });
        self.rebuild_graph();
        debug!("🧹 [PILLAR S] Registry synced. Active pools: {}", self.pool_registry.len());
    }

    fn rebuild_graph(&self) {
        let mut adj: FxHashMap<Address, Vec<PoolEdge>> = FxHashMap::default();
        let mut tokens_set = FxHashSet::default();
        let mut pool_list = Vec::new();

        for entry in self.state_mirror.pools.iter() {
            let pool_addr: &Address = entry.key();
            if let Some(reg) = self.pool_registry.get(pool_addr) {
                let (t0, t1, dex) = *reg;

                // Pillar Z: Autonomous Selection Logic
                let is_core = crate::constants::TOP_100_POOLS.contains(pool_addr);
                let is_hot = self.pool_activity.get(pool_addr)
                    .map(|t| t.elapsed().as_secs() < crate::constants::POOL_HOTNESS_TTL_SEC)
                    .unwrap_or(false);
                
                // Pillar Alpha: Grace period for new launches (30 mins)
                let is_alpha = self.alpha_grace_period.get(pool_addr)
                    .map(|t| t.elapsed().as_secs() < 1800) // 30 minutes
                    .unwrap_or(false);

                if !is_core && !is_hot && !is_alpha { continue; }

                // Focus strictly on Backrunning Aerodrome and Uniswap V3
                if dex != DexName::Aerodrome && dex != DexName::UniswapV3 {
                    continue;
                }

                pool_list.push(*pool_addr);
                tokens_set.insert(t0);
                tokens_set.insert(t1);
                
                adj.entry(t0).or_default().push(PoolEdge {
                    pool_address: *pool_addr, token_b: t1, dex_name: dex, ..Default::default()
                });
                adj.entry(t1).or_default().push(PoolEdge {
                    pool_address: *pool_addr, token_b: t0, dex_name: dex, ..Default::default()
                });
            }
        }

        let mut static_graph = StaticGraph::default();
        static_graph.tokens = tokens_set.into_iter().collect();
        for (i, &t) in static_graph.tokens.iter().enumerate() { static_graph.token_to_idx.insert(t, i as u32); }
        static_graph.pools = pool_list;
        for (i, &p) in static_graph.pools.iter().enumerate() { static_graph.pool_to_idx.insert(p, i as u32); }

        let mut current_offset = 0;
        for &token in &static_graph.tokens {
            static_graph.nodes.push(current_offset);
            if let Some(edges) = adj.get(&token) {
                for edge in edges {
                    static_graph.edges.push(StaticEdge {
                        pool_idx: *static_graph.pool_to_idx.get(&edge.pool_address).unwrap(),
                        target_token_idx: *static_graph.token_to_idx.get(&edge.token_b).unwrap(),
                        dex_name: edge.dex_name,
                    });
                    current_offset += 1;
                }
            }
        }
        static_graph.nodes.push(current_offset); // Sentinel

        info!("🕸️ [PILLAR C] Linearized Graph rebuilt with {} tokens and {} edges", static_graph.tokens.len(), static_graph.edges.len());
        self.graph.store(Arc::new(static_graph));
    }

    fn get_pool_from_router(&self, _router: &Address, t_in: &Address, t_out: &Address) -> Option<Address> {
        self.pool_registry.iter().find(|r| {
            let (p_t0, p_t1, _) = r.value();
            (p_t0 == t_in && p_t1 == t_out) || (p_t0 == t_out && p_t1 == t_in)
        }).map(|r| *r.key())
    }
}
