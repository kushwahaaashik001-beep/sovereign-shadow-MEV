#![allow(dead_code)]
use alloy_primitives::{Address, B256, U256, fixed_bytes};
use alloy::rpc::types::{Log, Filter};
use alloy::providers::{Provider, ProviderBuilder, WsConnect};
use crate::models::{Chain, SwapInfo, PoolKey, DexName, MempoolTx};
use futures_util::stream::StreamExt;
use dashmap::DashSet;
use rustc_hash::FxHasher;
use std::hash::BuildHasherDefault;
use rustc_hash::FxHashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tracing::{info, warn};
use crate::state_mirror::StateMirror;

#[derive(Clone)]
#[allow(dead_code)]
pub struct MempoolListenerConfig {
    pub endpoints: Vec<String>,
    pub stealth: bool,
    pub extra_endpoints: Vec<String>,
    pub worker_count: usize,
    pub fetcher_count: usize,
    pub tracked_pools: Arc<FxHashSet<PoolKey>>,
    pub use_txpool_content: bool,
    pub txpool_poll_interval_ms: u64,
    pub chain: Chain,
    pub min_gas_price_gwei: u64,
    pub heartbeat_interval_secs: Option<u64>,
    pub sequencer_endpoint: Option<String>,
}

impl Default for MempoolListenerConfig {
    fn default() -> Self {
        Self {
            endpoints: vec![],
            stealth: false,
            extra_endpoints: vec![],
            worker_count: 4,
            fetcher_count: 4,
            tracked_pools: Arc::new(FxHashSet::default()),
            use_txpool_content: false,
            txpool_poll_interval_ms: 200,
            chain: Chain::Base,
            min_gas_price_gwei: 0,
            heartbeat_interval_secs: Some(20),
            sequencer_endpoint: None,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SwapEvent {
    pub tx_hash: B256,
    pub sender: Address,
    pub swap_info: SwapInfo,
    pub effective_gas_price: U256,
    pub received_at: Instant,
    pub is_whale_trigger: bool,
    pub mempool_tx: Option<MempoolTx>,
}

#[derive(Debug, thiserror::Error)]
pub enum ListenerError {
    #[error("No endpoints available")]
    NoEndpoints,
    #[error("Other error: {0}")]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

pub struct MempoolListener {
    config: MempoolListenerConfig,
    event_tx: UnboundedSender<SwapEvent>,
    priority_tx: UnboundedSender<SwapEvent>,
    seen_hashes: Arc<DashSet<B256, BuildHasherDefault<FxHasher>>>,
}

impl MempoolListener {
    pub async fn new(
        config: MempoolListenerConfig,
        _grpc_tx: Option<UnboundedSender<SwapEvent>>,
    ) -> Result<(Self, UnboundedReceiver<SwapEvent>, UnboundedReceiver<SwapEvent>), ListenerError> {
        let (event_tx, event_rx) = unbounded_channel();
        let (priority_tx, priority_rx) = unbounded_channel();

        Ok((
            Self { 
                config, 
                event_tx, 
                priority_tx, 
                seen_hashes: Arc::new(DashSet::with_capacity_and_hasher(100_000, BuildHasherDefault::default())),
            },
            event_rx,
            priority_rx,
        ))
    }

    pub async fn run(self, mirror: Arc<StateMirror>) -> Result<(), ListenerError> {
        info!("📡 [BLOCK-WATCH] Disabling Mempool Snipe. Shifting to Event-Based Architecture (Zero Rate Limit)");
        info!("🥷 Mode: Back-running (Post-Swap Arbitrage)");
        
        // [STRICT-ISOLATION] Use only the primary log endpoint assigned to this listener
        let endpoint = self.config.endpoints.get(0).ok_or(ListenerError::NoEndpoints)?.clone();
        let event_tx = self.event_tx.clone();
        let seen_hashes_ws = self.seen_hashes.clone();

        loop {
            let ws = WsConnect::new(endpoint.clone());
            match ProviderBuilder::new().on_ws(ws).await {
                Ok(provider) => {
                    info!("✅ [EVENT-WATCH] Connected to Targeted Log Stream: {}", endpoint);
                    
                    // V2/V3/Aero Topics
                    let v2_sync = fixed_bytes!("1c411e9a96e071241c2f21f7726b17ae89e3cab4c78be50e062b03a9fffbbad1");
                    let v3_swap = fixed_bytes!("c42079f94a6350d7e6235f29174924f928cc2ac818eb64fed8004e115fbcca67");
                    
                    let filter = Filter::new().event_signature(vec![v2_sync, v3_swap]);
                    let log_sub = provider.subscribe_logs(&filter).await;

                    // Point #1: Delta Sync - Emitting readiness signal
                    if log_sub.is_ok() {
                        info!("🧠 [DELTA-SYNC] Live state monitoring active. No more multicall polling needed.");
                    }

                    if let Ok(logs) = log_sub {
                        let mut log_stream = logs.into_stream();
                        while let Some(log) = log_stream.next().await {
                            // [ZERO-POLLING] Update RAM State directly from the log
                            Self::update_mirror_state(&log, &mirror);
                            // Then trigger detection
                            Self::process_log_event(&log, &event_tx, &seen_hashes_ws);
                        }
                    }
                }
                Err(e) => warn!("⚠️ [SENTRY] Connection failed {}: {}", endpoint, e),
            }
            // Exponential backoff or simple delay
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    pub fn update_mirror_state(log: &Log, mirror: &StateMirror) {
        let v2_sync_topic = fixed_bytes!("1c411e9a96e071241c2f21f7726b17ae89e3cab4c78be50e062b03a9fffbbad1");
        let v3_swap_topic = fixed_bytes!("c42079f94a6350d7e6235f29174924f928cc2ac818eb64fed8004e115fbcca67");
        
        let pool_addr = log.address();
        if let Some(topic0) = log.topics().first() {
            if *topic0 == v2_sync_topic {
                // Point #2: Predictive State Mirroring (V2)
                // Sync event gives absolute reserves, no math needed. This is the ultimate Delta Sync.
                if log.data().data.len() >= 64 {
                    let r0 = U256::from_be_slice(&log.data().data[0..32]);
                    let r1 = U256::from_be_slice(&log.data().data[32..64]);
                    mirror.update_v2_reserves(pool_addr, r0, r1);
                }
            } else if *topic0 == v3_swap_topic {
                if let Some(state) = crate::v3_math::decode_v3_swap_log(log) {
                    mirror.update_v3_state(pool_addr, state.sqrt_price, state.tick, state.liquidity);
                }
            }
        }
    }

    fn process_log_event(
        log: &Log,
        event_tx: &UnboundedSender<SwapEvent>,
        seen_hashes: &DashSet<B256, BuildHasherDefault<FxHasher>>,
    ) {
        let tx_hash = match log.transaction_hash {
            Some(h) => h,
            None => return,
        };

        if !seen_hashes.insert(tx_hash) {
            return;
        }
        if seen_hashes.len() > 100_000 { seen_hashes.clear(); }

        // Pillar Z: Back-running Trigger
        // Hum Log address ko pool address ki tarah treat kar rahe hain. 
        // Engine ab state_mirror se latest reserves uthayega bina kisi extra RPC request ke.
        let event = SwapEvent {
            tx_hash,
            sender: Address::ZERO, 
            swap_info: SwapInfo {
                dex: DexName::UniswapV2, // Engine will resolve actual DEX from pool address
                router: log.address(),    // Using log address as the trigger pool
                token_in: Address::ZERO,
                token_out: Address::ZERO,
                amount_in: U256::ZERO,
                amount_out_min: U256::ZERO,
                to: Address::ZERO,
                fee: None,
                permit2_nonce: None,
            },
            effective_gas_price: U256::ZERO,
            received_at: Instant::now(),
            is_whale_trigger: false,
            mempool_tx: None,
        };

        let _ = event_tx.send(event);
    }
}
