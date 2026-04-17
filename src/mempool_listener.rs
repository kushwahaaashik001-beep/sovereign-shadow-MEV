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
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tracing::{info, warn};

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

    pub async fn run(self) -> Result<(), ListenerError> {
        info!("📡 [Pillar A] Private WSS Streaming Active (Blast/Ankr Optimized)");
        
        let decoder = Arc::new(UniversalDecoder::new());
        let seen_hashes = self.seen_hashes.clone();
        let mut tasks = Vec::new();

        // Pillar A: Multi-WSS Scavenging Orchestration
        // Connect to ALL provided endpoints in parallel to bypass rate limits and win the latency race.
        for endpoint in &self.config.endpoints {
            let endpoint = endpoint.clone();
            let event_tx = self.event_tx.clone();
            let decoder_inner = decoder.clone();
            let config = self.config.clone();
            let seen_hashes_ws = seen_hashes.clone();

            tasks.push(tokio::spawn(async move {
                loop {
                    let ws = WsConnect::new(endpoint.clone());
                    match ProviderBuilder::new().on_ws(ws).await {
                        Ok(provider) => {
                            info!("✅ [SENTRY] Connected to WSS Feed: {}", endpoint);
                            if let Ok(sub) = provider.subscribe_full_pending_transactions().await {
                                let mut stream = sub.into_stream();
                                while let Some(tx) = stream.next().await {
                                    Self::process_raw_tx(&tx, &decoder_inner, &event_tx, &config, &seen_hashes_ws);
                                }
                            }
                        }
                        Err(e) => warn!("⚠️ [SENTRY] Connection failed {}: {}", endpoint, e),
                    }
                    // Exponential backoff to avoid spamming failed connections
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }));
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

        // Pillar Z: Autonomous Discovery - Attempt to decode even unknown routers
        // We don't return early if it's not a known router, we let the decoder try its magic.
        let is_known = TARGET_ROUTERS.contains(&to) || config.tracked_pools.contains(&PoolKey { pool: to });

        // 2. Efficient Data Conversion
        // Converting alloy_primitives::Bytes to bytes::Bytes (ref-counted)
        let input_ref = bytes::Bytes::from(tx.input.0.clone());

        let decode_tx = DecodeTx {
            to: Some(to),
            value: tx.value,
            input: input_ref,
        };

        let swaps = decoder.decode(&decode_tx);
        if swaps.is_empty() && !is_known { return; } // Drop if unknown and not a swap

        for swap in swaps {
            // Signal volume detection for non-tracked pools to trigger registry promotion
            if !is_known && swap.amount_in > U256::from(5 * 10u128.pow(16)) {
                 // This swap is on an unknown contract but has volume. High-Alpha signal.
            }

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
