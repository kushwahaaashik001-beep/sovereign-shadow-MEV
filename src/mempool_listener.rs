use crate::models::{SwapInfo, PoolKey};
use crate::universal_decoder::UniversalDecoder;
use dashmap::DashMap;
use arc_swap::ArcSwap;
use ethers::{
    prelude::*,
    providers::{Provider, Ws},
    types::{Transaction, H256, U256},
};
use futures_util::stream::{SelectAll, Stream};
use futures_util::StreamExt;
use rand::Rng;
use rustc_hash::FxHashSet;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel, UnboundedReceiver};
use tokio::time;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::{debug, error, info, warn};

#[derive(Clone)]
#[allow(dead_code)]
pub struct MempoolListenerConfig {
    pub endpoints:              Vec<String>,
    pub stealth:                bool,
    pub extra_endpoints:        Vec<String>,
    pub worker_count:           usize,
    pub fetcher_count:          usize,
    pub tracked_pools:          Arc<FxHashSet<PoolKey>>,
    pub use_txpool_content:     bool,
    pub txpool_poll_interval_ms: u64,
    pub chain:                  Chain,
    pub min_gas_price_gwei:     u64,
    pub heartbeat_interval_secs: Option<u64>,
    pub sequencer_endpoint:     Option<String>,
}

impl Default for MempoolListenerConfig {
    fn default() -> Self {
        Self {
            endpoints:              vec![],
            stealth:                false,
            extra_endpoints:        vec![],
            worker_count:           8,
            fetcher_count:          16,
            tracked_pools:          Arc::new(FxHashSet::default()),
            use_txpool_content:     false,
            txpool_poll_interval_ms: 200,
            chain:                  Chain::Mainnet,
            min_gas_price_gwei:     0,
            heartbeat_interval_secs: Some(20),
            sequencer_endpoint:     None,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SwapEvent {
    pub tx_hash:             H256,
    pub sender:              Address,
    pub swap_info:           SwapInfo,
    pub effective_gas_price: U256,
    pub received_at:         Instant,
    pub is_whale_trigger:    bool,
}

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum ListenerError {
    #[error("Provider error: {0}")]
    Provider(#[from] ProviderError),
    #[error("Decoding error: {0}")]
    Decode(String),
    #[error("No endpoints available")]
    NoEndpoints,
    #[error("Stream ended")]
    StreamEnded,
    #[error("Heartbeat failed")]
    HeartbeatFailed,
    #[error("Other error: {0}")]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

struct ProviderPool {
    providers: Vec<Arc<Provider<Ws>>>,
    next:      AtomicUsize,
}

impl ProviderPool {
    fn new(providers: Vec<Arc<Provider<Ws>>>) -> Self {
        Self { providers, next: AtomicUsize::new(0) }
    }
    fn next(&self) -> Arc<Provider<Ws>> {
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.providers.len();
        self.providers[idx].clone()
    }
}

struct BaseFeeCache {
    value: Arc<ArcSwap<Option<U256>>>,
}
impl BaseFeeCache {
    fn new() -> Self { Self { value: Arc::new(ArcSwap::from_pointee(None)) } }
    fn update(&self, v: Option<U256>) { self.value.store(Arc::new(v)); }
    fn get(&self) -> Option<U256> { **self.value.load() }
}

struct ListenerStats {
    txs_seen:       AtomicU64,
    txs_deduped:    AtomicU64,
    swaps_detected: AtomicU64,
    errors:         AtomicU64,
    last_rotation:  AtomicU64,
    reconnects:     AtomicU64,
}
impl ListenerStats {
    fn new() -> Self {
        Self {
            txs_seen:       AtomicU64::new(0),
            txs_deduped:    AtomicU64::new(0),
            swaps_detected: AtomicU64::new(0),
            errors:         AtomicU64::new(0),
            last_rotation:  AtomicU64::new(0),
            reconnects:     AtomicU64::new(0),
        }
    }
}

pub struct MempoolListener {
    config:           MempoolListenerConfig,
    event_tx:         UnboundedSender<SwapEvent>,
    priority_tx:      UnboundedSender<SwapEvent>,
    provider_pool:    Arc<ProviderPool>,
    stream_selector:  SelectAll<Pin<Box<dyn Stream<Item = Transaction> + Send>>>,
    stats:            Arc<ListenerStats>,
    tx_hash_cache:    Arc<DashMap<H256, Instant>>,
    stream_sender:    UnboundedSender<Pin<Box<dyn Stream<Item = Transaction> + Send>>>,
    base_fee_cache:   Arc<BaseFeeCache>,
    worker_txs:       Vec<UnboundedSender<(Transaction, Instant)>>,
    worker_rxs:       Vec<UnboundedReceiver<(Transaction, Instant)>>,
    decoder:          Arc<UniversalDecoder>,
}

impl MempoolListener {
    pub async fn new(
        config: MempoolListenerConfig,
    ) -> Result<(Self, UnboundedReceiver<SwapEvent>, UnboundedReceiver<SwapEvent>), ListenerError> {
        let (event_tx,    event_rx)    = unbounded_channel();
        let (priority_tx, priority_rx) = unbounded_channel();
        let (stream_sender, _)         = unbounded_channel::<Pin<Box<dyn Stream<Item = Transaction> + Send>>>();

        let mut all_endpoints = config.endpoints.clone();
        if config.stealth { all_endpoints.extend(config.extra_endpoints.clone()); }
        if all_endpoints.is_empty() { return Err(ListenerError::NoEndpoints); }

        let mut providers = Vec::new();
        for ep in &all_endpoints {
            match Provider::<Ws>::connect(ep).await {
                Ok(p)  => providers.push(Arc::new(p)),
                Err(e) => warn!("Failed to connect to {}: {}", ep, e),
            }
        }
        if providers.is_empty() { return Err(ListenerError::NoEndpoints); }
        let provider_pool = Arc::new(ProviderPool::new(providers));

        let stats          = Arc::new(ListenerStats::new());
        let base_fee_cache = Arc::new(BaseFeeCache::new());

        let mut stream_selector = SelectAll::new();
        let stream = Self::create_block_tx_stream(provider_pool.next(), config.fetcher_count).await?;
        stream_selector.push(stream);

        let mut worker_txs = Vec::with_capacity(config.worker_count);
        let mut worker_rxs = Vec::with_capacity(config.worker_count);
        for _ in 0..config.worker_count {
            let (tx, rx) = unbounded_channel::<(Transaction, Instant)>();
            worker_txs.push(tx);
            worker_rxs.push(rx);
        }

        Ok((
            Self {
                config: config.clone(),
                event_tx, priority_tx,
                provider_pool,
                stream_selector, stats,
                tx_hash_cache: Arc::new(DashMap::new()),
                stream_sender, base_fee_cache,
                worker_txs, worker_rxs,
                decoder: Arc::new(UniversalDecoder::new()),
            },
            event_rx,
            priority_rx,
        ))
    }

    /// Block-based TX stream — works on Anvil AND real Base mainnet.
    /// Fetches ALL transactions from every new block.
    /// Captures V2 + V3 + Universal Router swaps.
    async fn create_block_tx_stream(
        provider:      Arc<Provider<Ws>>,
        fetcher_count: usize,
    ) -> Result<Pin<Box<dyn Stream<Item = Transaction> + Send>>, ListenerError> {
        info!("🔌 [PILLAR A] Block-based TX stream starting (concurrency={})...", fetcher_count);

        let (tx_out, tx_in) = unbounded_channel::<Transaction>();
        let p   = provider.clone();
        let t   = tx_out.clone();

        tokio::spawn(async move {
            loop {
                match p.subscribe_blocks().await {
                    Ok(mut stream) => {
                        info!("✅ [PILLAR A] Block subscription live — capturing ALL txs per block");
                        let sem = Arc::new(tokio::sync::Semaphore::new(fetcher_count));
                        while let Some(block) = stream.next().await {
                            let hash = match block.hash { Some(h) => h, None => continue };
                            let p2   = p.clone();
                            let t2   = t.clone();
                            let permit = match sem.clone().try_acquire_owned() {
                                Ok(p)  => p,
                                Err(_) => continue, // saturated — skip block
                            };
                            tokio::spawn(async move {
                                if let Ok(Some(full)) = p2.get_block_with_txs(hash).await {
                                    for tx in full.transactions {
                                        let _ = t2.send(tx);
                                    }
                                }
                                drop(permit);
                            });
                        }
                        warn!("[PILLAR A] Block stream ended — reconnecting in 1s");
                    }
                    Err(e) => warn!("[PILLAR A] subscribe_blocks error: {} — retrying in 1s", e),
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });

        Ok(Box::pin(UnboundedReceiverStream::new(tx_in)))
    }

    pub async fn run(mut self) -> Result<(), ListenerError> {
        info!("Mempool listener started (stealth={})", self.config.stealth);

        let (stream_sender, mut stream_receiver) = unbounded_channel();
        self.stream_sender = stream_sender;

        self.spawn_block_listener();
        let worker_rxs = std::mem::take(&mut self.worker_rxs);
        self.spawn_workers(worker_rxs);

        if self.config.stealth && self.provider_pool.providers.len() > 1 {
            self.spawn_rotator();
        }
        if let Some(interval) = self.config.heartbeat_interval_secs {
            self.spawn_heartbeat(interval);
        }

        // Cache cleanup
        let cache = self.tx_hash_cache.clone();
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(15));
            loop {
                interval.tick().await;
                let now = Instant::now();
                cache.retain(|_, &mut v| now.duration_since(v) < Duration::from_secs(15));
            }
        });

        // Stats logger
        let stats_clone = self.stats.clone();
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                info!(
                    "Mempool Stats - Seen: {}, Deduped: {}, Swaps: {}, Errors: {}",
                    stats_clone.txs_seen.load(Ordering::Relaxed),
                    stats_clone.txs_deduped.load(Ordering::Relaxed),
                    stats_clone.swaps_detected.load(Ordering::Relaxed),
                    stats_clone.errors.load(Ordering::Relaxed),
                );
            }
        });

        let mut current_worker = 0;
        let worker_count = self.worker_txs.len();

        loop {
            tokio::select! {
                Some(_) = stream_receiver.recv() => {
                    info!("New mempool stream added");
                }
                Some(tx) = self.stream_selector.next() => {
                    if self.tx_hash_cache.contains_key(&tx.hash) {
                        self.stats.txs_deduped.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    let received_at = Instant::now();
                    self.tx_hash_cache.insert(tx.hash, received_at);
                    self.stats.txs_seen.fetch_add(1, Ordering::Relaxed);

                    if self.worker_txs[current_worker].send((tx, received_at)).is_err() {
                        error!("Worker channel {} closed", current_worker);
                    }
                    current_worker = (current_worker + 1) % worker_count;
                }
                else => {
                    warn!("All streams ended, reconnecting...");
                    time::sleep(Duration::from_secs(1)).await;
                    if let Ok(stream) = Self::create_block_tx_stream(
                        self.provider_pool.next(), self.config.fetcher_count
                    ).await {
                        self.stream_selector.push(stream);
                        self.stats.reconnects.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
    }

    fn spawn_block_listener(&self) -> tokio::task::JoinHandle<()> {
        let provider       = self.provider_pool.next();
        let base_fee_cache = self.base_fee_cache.clone();
        tokio::spawn(async move {
            if let Ok(mut stream) = provider.subscribe_blocks().await {
                while let Some(block) = stream.next().await {
                    base_fee_cache.update(block.base_fee_per_gas);
                }
            }
        })
    }

    fn spawn_workers(
        &self,
        mut worker_rxs: Vec<UnboundedReceiver<(Transaction, Instant)>>,
    ) -> Vec<tokio::task::JoinHandle<()>> {
        let mut handles = Vec::new();
        let count = worker_rxs.len();

        for i in 0..count {
            let event_tx       = self.event_tx.clone();
            let priority_tx    = self.priority_tx.clone();
            let tracked        = self.config.tracked_pools.clone();
            let stats          = self.stats.clone();
            let min_gas        = self.config.min_gas_price_gwei;
            let base_fee_cache = self.base_fee_cache.clone();
            let decoder        = self.decoder.clone();
            let mut rx         = worker_rxs.remove(0);

            let handle = tokio::spawn(async move {
                info!("Worker {} started (Lock-Free)", i);
                while let Some((tx, received_at)) = rx.recv().await {
                    let hash = tx.hash;
                    if let Err(e) = Self::process_transaction(
                        &tx, tracked.clone(),
                        event_tx.clone(), priority_tx.clone(),
                        stats.clone(), min_gas,
                        base_fee_cache.clone(), received_at,
                        decoder.clone(),
                    ).await {
                        stats.errors.fetch_add(1, Ordering::Relaxed);
                        debug!("Error processing tx {}: {}", hash, e);
                    }
                }
                warn!("Worker {} shutting down", i);
            });
            handles.push(handle);
        }
        handles
    }

    fn spawn_rotator(&self) -> tokio::task::JoinHandle<()> {
        let config        = self.config.clone();
        let stats         = self.stats.clone();
        let provider_pool = self.provider_pool.providers.clone();
        let stream_sender = self.stream_sender.clone();

        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                if !config.stealth { continue; }
                let now  = Instant::now().elapsed().as_secs();
                let last = stats.last_rotation.load(Ordering::Relaxed);
                if now - last < 60 { continue; }

                let do_rotate = {
                    let mut rng = rand::thread_rng();
                    rng.gen_bool(0.05)
                };
                if do_rotate {
                    let idx      = { let mut rng = rand::thread_rng(); rng.gen_range(0..provider_pool.len()) };
                    let provider = provider_pool[idx].clone();
                    if let Ok(stream) = Self::create_block_tx_stream(provider, 16).await {
                        let _ = stream_sender.send(stream);
                        stats.last_rotation.store(now, Ordering::Relaxed);
                        info!("Rotated to new RPC provider");
                    }
                }
            }
        })
    }

    fn spawn_heartbeat(&self, interval_secs: u64) -> tokio::task::JoinHandle<()> {
        let provider = self.provider_pool.next();
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(interval_secs));
            loop {
                interval.tick().await;
                match provider.get_block_number().await {
                    Ok(_)  => debug!("Heartbeat OK"),
                    Err(e) => warn!("Heartbeat failed: {}", e),
                }
            }
        })
    }

    fn effective_gas_price(tx: &Transaction, base_fee: Option<U256>) -> U256 {
        if let Some(price) = tx.gas_price { return price; }
        if let (Some(max_fee), Some(priority), Some(base)) =
            (tx.max_fee_per_gas, tx.max_priority_fee_per_gas, base_fee)
        {
            let proposed = base + priority;
            return if max_fee < proposed { max_fee } else { proposed };
        }
        tx.max_fee_per_gas.unwrap_or(U256::zero())
    }

    async fn process_transaction(
        tx:             &Transaction,
        tracked_pools:  Arc<FxHashSet<PoolKey>>,
        event_tx:       UnboundedSender<SwapEvent>,
        priority_tx:    UnboundedSender<SwapEvent>,
        stats:          Arc<ListenerStats>,
        min_gas_gwei:   u64,
        base_fee_cache: Arc<BaseFeeCache>,
        received_at:    Instant,
        decoder:        Arc<UniversalDecoder>,
    ) -> Result<(), ListenerError> {
        let base_fee     = base_fee_cache.get();
        let effective_gas = Self::effective_gas_price(tx, base_fee);

        if effective_gas < U256::from(min_gas_gwei * 1_000_000_000) {
            return Ok(());
        }

        // Must have a recipient
        let _to = match tx.to { Some(a) => a, None => return Ok(()) };

        // Must have calldata (swap functions need at least 4 bytes)
        if tx.input.0.len() < 4 { return Ok(()); }

        // Decode swaps from this transaction
        let swaps = decoder.decode_transaction_deep(tx);
        for swap_res in swaps {
            let swap: SwapInfo = match swap_res {
                Ok(s)  => s,
                Err(_) => { stats.errors.fetch_add(1, Ordering::Relaxed); continue; }
            };

            if !tracked_pools.is_empty() && !swap.is_tracked(&tracked_pools) {
                continue;
            }

            stats.swaps_detected.fetch_add(1, Ordering::Relaxed);

            // Whale detection: ~$100k at $2600/ETH
            const WHALE_WEI: u128 = 38_461_538_461_538_461_538;
            let is_whale = swap.amount_in >= U256::from(WHALE_WEI);

            let event = SwapEvent {
                tx_hash:             tx.hash,
                sender:              tx.from,
                swap_info:           swap,
                effective_gas_price: effective_gas,
                received_at,
                is_whale_trigger:    is_whale,
            };

            if is_whale {
                let _ = priority_tx.send(event);
            } else {
                let _ = event_tx.send(event);
            }
        }

        Ok(())
    }
}
