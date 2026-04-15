#![allow(dead_code)]
use alloy_primitives::{Address, B256, U256};
use alloy::rpc::types::Transaction as AlloyTransaction;
use alloy::providers::{Provider, ProviderBuilder, WsConnect};
use crate::models::{Chain, SwapInfo, PoolKey, MempoolTx};
use crate::universal_decoder::{UniversalDecoder, DecodeTx};
use crate::constants::TARGET_ROUTERS;
use futures_util::stream::StreamExt;
use dashmap::DashSet;
use rustc_hash::FxHasher;
use std::hash::BuildHasherDefault;
use rustc_hash::FxHashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel, UnboundedReceiver};
use tracing::{info, warn, debug};

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
            chain: Chain::Mainnet,
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
    p2p_tx: UnboundedSender<AlloyTransaction>,
    p2p_rx: UnboundedReceiver<AlloyTransaction>,
    seen_hashes: Arc<DashSet<B256, BuildHasherDefault<FxHasher>>>,
}

impl MempoolListener {
    pub async fn new(
        config: MempoolListenerConfig,
        _grpc_tx: Option<UnboundedSender<SwapEvent>>,
    ) -> Result<(Self, UnboundedReceiver<SwapEvent>, UnboundedReceiver<SwapEvent>), ListenerError> {
        let (event_tx, event_rx) = unbounded_channel();
        let (priority_tx, priority_rx) = unbounded_channel();
        let (p2p_tx, p2p_rx) = unbounded_channel();
        Ok((
            Self { 
                config, 
                event_tx, 
                priority_tx, 
                p2p_tx, 
                p2p_rx,
                seen_hashes: Arc::new(DashSet::with_capacity_and_hasher(100_000, BuildHasherDefault::default())),
            },
            event_rx,
            priority_rx,
        ))
    }

    pub fn p2p_tx(&self) -> UnboundedSender<AlloyTransaction> {
        self.p2p_tx.clone()
    }

    pub async fn run(self) -> Result<(), ListenerError> {
        info!("🚀 [Pillar A] Mempool Surveillance Active on {:?}", self.config.chain);
        
        let decoder = Arc::new(UniversalDecoder::new());
        let seen_hashes = self.seen_hashes.clone();
        let mut tasks = Vec::new();

        // Pillar T: Integrated P2P Sentry Stream (Zero-latency direct feed)
        let mut p2p_rx = self.p2p_rx;
        let event_tx_p2p = self.event_tx.clone();
        let decoder_p2p = decoder.clone();
        let config_p2p = self.config.clone();
        let seen_hashes_p2p = seen_hashes.clone();
        tasks.push(tokio::spawn(async move {
            while let Some(tx) = p2p_rx.recv().await {
                Self::process_raw_tx(&tx, &decoder_p2p, &event_tx_p2p, &config_p2p, &seen_hashes_p2p);
            }
        }));

        // Pillar T: txpool_content Polling (Secondary feed for hidden txs)
        if self.config.use_txpool_content {
            let event_tx = self.event_tx.clone();
            let decoder = decoder.clone();
            let config = self.config.clone();
            let seen_hashes_poll = seen_hashes.clone();
            
            if let Some(endpoint) = self.config.endpoints.first() {
                let endpoint = endpoint.clone();
                tasks.push(tokio::spawn(async move {
                    if let Ok(url) = endpoint.parse() {
                        let provider = Arc::new(ProviderBuilder::new().on_http(url));
                        let mut interval = tokio::time::interval(Duration::from_millis(config.txpool_poll_interval_ms));
                        loop {
                            interval.tick().await;
                            let res: Result<serde_json::Value, _> = provider.raw_request("txpool_content".into(), Vec::<String>::new()).await;
                            if let Ok(content) = res {
                                if let Some(pending) = content.get("pending").and_then(|p| p.as_object()) {
                                    for txs in pending.values() {
                                        if let Some(tx_map) = txs.as_object() {
                                            for tx_val in tx_map.values() {
                                                if let Ok(tx) = serde_json::from_value::<AlloyTransaction>(tx_val.clone()) {
                                                    Self::process_raw_tx(&tx, &decoder, &event_tx, &config, &seen_hashes_poll);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }));
            }
        }

        for endpoint in &self.config.endpoints {
            let endpoint = endpoint.clone();
            let event_tx = self.event_tx.clone();
            let decoder = decoder.clone();
            let config = self.config.clone();
            let seen_hashes_ws = seen_hashes.clone();

            let task = tokio::spawn(async move {
                loop {
                    debug!("📡 Attempting connection to: {}", endpoint);
                    if let Ok(ws) = Ok::<WsConnect, ListenerError>(WsConnect::new(endpoint.clone())) {
                        if let Ok(provider) = ProviderBuilder::new().on_ws(ws).await {
                            info!("✅ Connected to mempool feed: {}", endpoint);
                            
                            // Optimization: Try full transaction streaming first
                            if let Ok(sub) = provider.subscribe_full_pending_transactions().await {
                                let mut stream = sub.into_stream();
                                while let Some(tx) = stream.next().await {
                                    Self::process_raw_tx(&tx, &decoder, &event_tx, &config, &seen_hashes_ws);
                                }
                            } else if let Ok(sub) = provider.subscribe_pending_transactions().await {
                                // Fallback: Hash stream + Parallel Fetching
                                let mut stream = sub.into_stream();
                                while let Some(hash) = stream.next().await {
                                    let p = provider.clone();
                                    let d = decoder.clone();
                                    let et = event_tx.clone();
                                    let c = config.clone();
                                    let sh = seen_hashes_ws.clone();
                                    tokio::spawn(async move {
                                        if let Ok(Some(tx)) = p.get_transaction_by_hash(hash).await {
                                            Self::process_raw_tx(&tx, &d, &et, &c, &sh);
                                        }
                                    });
                                }
                            }
                        }
                    }
                    warn!("🔄 Mempool connection lost [{}]. Retrying in 5s...", endpoint);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            });
            tasks.push(task);
        }

        // Keep the main loop alive
        for task in tasks {
            let _ = task.await;
        }
        Ok(())
    }

    #[inline(always)]
    fn process_raw_tx(
        tx: &AlloyTransaction,
        decoder: &UniversalDecoder,
        event_tx: &UnboundedSender<SwapEvent>,
        config: &MempoolListenerConfig,
        seen_hashes: &DashSet<B256, BuildHasherDefault<FxHasher>>,
    ) {
        // 0. Deduplication Filter: Skip if already processed (P2P wins usually)
        if !seen_hashes.insert(tx.hash) {
            return;
        }
        if seen_hashes.len() > 100_000 { seen_hashes.clear(); }

        // 1. Zero-Cost Pre-Filter: Check if 'to' is a relevant contract
        let to = match tx.to {
            Some(addr) => addr,
            None => return,
        };

        if !TARGET_ROUTERS.contains(&to) && !config.tracked_pools.contains(&PoolKey { pool: to }) {
            return;
        }

        // 2. Efficient Data Conversion
        // Converting alloy_primitives::Bytes to bytes::Bytes (ref-counted)
        let input_ref = bytes::Bytes::from(tx.input.0.clone());

        let decode_tx = DecodeTx {
            to: Some(to),
            value: tx.value,
            input: input_ref,
        };

        let swaps = decoder.decode(&decode_tx);
        for swap in swaps {
            let event = SwapEvent {
                tx_hash: tx.hash,
                sender: tx.from,
                swap_info: swap,
                effective_gas_price: U256::from(tx.gas_price.unwrap_or_default()),
                received_at: Instant::now(),
                is_whale_trigger: tx.value > U256::from(10u128.pow(18)), // 1 ETH trigger
                mempool_tx: Some(MempoolTx {
                    data: tx.input.clone(),
                    hash: tx.hash,
                    to: tx.to,
                }),
            };
            // Send to detector channel
            let _ = event_tx.send(event);
        }
    }
}
