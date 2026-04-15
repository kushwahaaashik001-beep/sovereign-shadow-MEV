use the_sovereign_shadow::arbitrage_detector::{ArbitrageDetector, DetectorConfig};
use the_sovereign_shadow::bundle_builder::{BundleBuilder, BundleBuilderConfig};
use the_sovereign_shadow::bidding_engine::BiddingEngine;
use the_sovereign_shadow::flash_loan_executor::FlashLoanExecutor;
use the_sovereign_shadow::gas_feed::GasPriceFeed;
use the_sovereign_shadow::nonce_manager::NonceManager;
use the_sovereign_shadow::profit_manager::ProfitManager;
use the_sovereign_shadow::inventory_manager::InventoryManager;
use the_sovereign_shadow::state_mirror::StateMirror;
use the_sovereign_shadow::discovery::Discovery;
use the_sovereign_shadow::factory_scanner::{FactoryScanner, NewPoolEvent};
use the_sovereign_shadow::state_simulator::StateSimulator;
use the_sovereign_shadow::mempool_listener::{MempoolListener, MempoolListenerConfig};
use the_sovereign_shadow::utils::{CircuitBreaker, L1DataFeeCalculator};
use the_sovereign_shadow::rpc_manager::RpcManager;
use the_sovereign_shadow::{constants, telemetry, WsProviderPool};
use dotenv::dotenv;
use alloy::providers::{ProviderBuilder, WsConnect, Provider};
use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, U256};
use std::error::Error;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc};
use futures_util::StreamExt;
use std::env;
use tracing::{error, info, warn};
use tokio_stream::wrappers::ReceiverStream;
use dashmap::DashSet;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, filter::LevelFilter};

// gRPC Imports
pub mod hydra {
    tonic::include_proto!("hydra");
}
use hydra::hydra_network_server::{HydraNetwork, HydraNetworkServer};
use hydra::{RawOpportunity, Empty};
use tonic::{transport::Server, Request, Response, Status, Streaming, metadata::MetadataValue};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("sniper");

    let core_ids = core_affinity::get_core_ids().expect("Failed to get core IDs");
    let core_counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(core_ids.len())
        .enable_all()
        .on_thread_start(move || {
            let idx = core_counter.fetch_add(1, Ordering::SeqCst) % core_ids.len();
            core_affinity::set_for_current(core_ids[idx]);
        })
        .build()?;

    match mode {
        "scout" => {
            info!("👁️ Space A: Global Scout Mode Starting...");
            runtime.block_on(run_scout())
        }
        _ => {
            info!("🎯 Space B: Sniper Brain Mode Starting...");
            runtime.block_on(run_engine())
        }
    }
}

async fn run_engine() -> Result<(), Box<dyn Error>> {
    dotenv().ok();

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env().add_directive(LevelFilter::INFO.into()))
        .init();

    info!("🛸 Sovereign Shadow MEV Engine — BEAST MODE ONLINE");

    let chain_name = env::var("CHAIN").unwrap_or_else(|_| "base".to_string());
    let chain = match chain_name.as_str() {
        "base"     => the_sovereign_shadow::models::Chain::Base,
        "arbitrum" => the_sovereign_shadow::models::Chain::Arbitrum,
        _          => the_sovereign_shadow::models::Chain::Mainnet,
    };
    info!("⛓️  Chain: {:?}", chain);

    // Pillar S: Strategic Env Loading - Sniper needs execution keys
    // Logic: Split by comma to support multiple keys (Rotation logic from v1.0)
    // Fallback logic for variable names (checks for SHADOW_WS_URL or SHADOW_WS_URL_1)
    let wss_raw = env::var("SHADOW_WS_URL")
        .or_else(|_| env::var("SHADOW_WS_URL_1"))
        .expect("🚀 Sniper needs SHADOW_WS_URL or SHADOW_WS_URL_1");

    let wss_urls: Vec<String> = wss_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();

    let rpc_raw = env::var("SHADOW_RPC_URL")
        .or_else(|_| env::var("SHADOW_RPC_URL_1"))
        .expect("🚀 Sniper needs SHADOW_RPC_URL or SHADOW_RPC_URL_1");

    let http_urls: Vec<String> = rpc_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();

    let priv_key_raw = env::var("SHADOW_PRIVATE_KEY").expect("🚀 Sniper Mode needs SHADOW_PRIVATE_KEY");
    let priv_key = priv_key_raw.trim().trim_start_matches("0x");

    // Initialize WSS Provider Pool for parallel listening
    let provider_futures = wss_urls.iter().map(|url| {
        let url = url.clone();
        async move {
            let ws = WsConnect::new(url.clone());
            match ProviderBuilder::new().on_ws(ws).await {
                Ok(p) => Some(Arc::new(p.boxed())),
                Err(e) => { error!("❌ [INFRA] Connection failed {}: {}", url, e); None }
            }
        }
    });
    let ws_providers: Vec<Arc<the_sovereign_shadow::WsProvider>> = futures::future::join_all(provider_futures).await.into_iter().flatten().collect();

    if ws_providers.is_empty() { return Err("No valid WSS endpoints".into()); }
    let ws_provider_pool = Arc::new(WsProviderPool::new(ws_providers));
    
    // Initialize HTTP RPC Manager for rotated simulations
    let rpc_manager = Arc::new(RpcManager::new(http_urls));
    
    // Initialize Telemetry Nervous System
    let (tele_tx, tele_rx) = mpsc::unbounded_channel();
    let telemetry_handle = Arc::new(telemetry::TelemetryHandle::new(tele_tx));
    let tele_handle_for_loop = telemetry_handle.clone();
    tokio::spawn(async move {
        telemetry::run_telemetry_loop(tele_rx).await;
    });

    // Primary provider for setup (uses the first available key)
    let ws_provider = ws_provider_pool.next();
    let http_provider = rpc_manager.get_next_provider();

    let executor_address: Address = env::var("EXECUTOR_ADDRESS")
        .unwrap_or_default().parse().unwrap_or(Address::ZERO);

    let chain_id = ws_provider.get_chain_id().await?;
    info!("🔗 Chain ID: {}", chain_id);

    let wallet = PrivateKeySigner::from_str(priv_key)?;
    info!("👛 Wallet: {:?}", wallet.address());

    let circuit_breaker = Arc::new(CircuitBreaker::new(5, 30));
    let gas_feed = Arc::new(GasPriceFeed::new(ws_provider_pool.clone(), chain).await);

    let state_mirror = StateMirror::new();
    let bidding_engine = Arc::new(BiddingEngine::new(state_mirror.clone()));

    let (pool_tx, _) = broadcast::channel::<NewPoolEvent>(2048);
    {
        let scanner = Arc::new(FactoryScanner::new(ws_provider_pool.clone(), pool_tx.clone(), chain));
        tokio::spawn(async move { scanner.run().await; });
    }

    let nonce_manager = Arc::new(NonceManager::new(ws_provider.clone(), wallet.address()).await?);

    let relays = vec![
        "https://relay.flashbots.net".to_string(),
        "https://rpc.beaverbuild.org/".to_string(),
    ];
    let l2_rpcs: Vec<String> = env::var("PRIVATE_RPCS")
        .unwrap_or_default().split(',')
        .filter(|s| !s.is_empty()).map(String::from).collect();

    let identity_key_raw = env::var("FLASHBOTS_IDENTITY_KEY").unwrap_or(priv_key_raw.clone());
    let identity_wallet = PrivateKeySigner::from_str(identity_key_raw.trim().trim_start_matches("0x")).unwrap_or(wallet.clone());

    let state_simulator = Arc::new(StateSimulator::new(state_mirror.clone()));

    let bundle_builder = Arc::new(BundleBuilder::new(
        BundleBuilderConfig {
            chain_id,
            chain,
            signer:             wallet.clone(),
            identity_signer:    identity_wallet,
            executor_address,
            min_profit_eth:     U256::from(10u64.pow(14)),
            relays,
            l2_private_rpcs:    l2_rpcs.clone(),
            base_bribe_percent: 90,
            max_gas_price_gwei: 100,
            enable_simulation: true,
            use_flashbots_simulation: false,
            check_flash_loan: false,
            relay_timeout_ms:   500,
            stealth_jitter:     true,
            use_raw_encoding: false,
            nonce_recovery_blocks: 10,
            max_consecutive_failures: 5,
            pause_duration_secs: 60,
            ai_strategy: None,
            telemetry_tx: Some(tele_handle_for_loop),
        },
        ws_provider.clone(),
        nonce_manager.clone(),
        circuit_breaker.clone(),
        state_simulator.clone(),
    ).await?);

    // Pillar S: Self-Healing Memory Management (8GB RAM Safety)
    {
        let mirror = state_mirror.clone();
        let provider = http_provider.clone(); // Using HTTP for heavy batch reads
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop { interval.tick().await; mirror.prune_stale_pools(100); }
        });

        // Pillar B: Background Batch State Sync (Multicall3)
        // This fills the gap: keeping 3000+ pools synchronized in the background
        let mirror_sync = state_mirror.clone();
        let provider_sync = provider.clone();
        tokio::spawn(async move {
            info!("🔄 [STATE] Background Multicall Sync started");
            // Initial heavy sync
            let _ = mirror_sync.sync_all_pools_multicall(provider_sync.clone()).await;
            
            let mut interval = tokio::time::interval(Duration::from_secs(4)); // Every ~2 blocks on Base
            loop {
                interval.tick().await;
                if let Err(e) = mirror_sync.sync_all_pools_multicall(provider_sync.clone()).await {
                    error!("❌ [STATE SYNC] Multicall batch sync failed: {}", e);
                }
            }
        });
    }

    // Pillar E: Real-time Block Tracker & State Mirror Sync (Heartbeat)
    // This task ensures all bundles target the correct next block
    {
        let cb_sync = circuit_breaker.clone();
        let mirror_sync = state_mirror.clone();
        let bb_sync = bundle_builder.clone();
        let ws_pool = ws_provider_pool.clone();
        tokio::spawn(async move {
            loop {
                let provider = ws_pool.next();
                if let Ok(sub) = provider.subscribe_blocks().await {
                    let mut stream = sub.into_stream();
                    while let Some(block) = stream.next().await {
                        let block_number = block.header.number;
                        let base_fee = U256::from(block.header.base_fee_per_gas.unwrap_or_default());
                        let timestamp = block.header.timestamp;
                        
                        mirror_sync.sync_block(block_number, base_fee, timestamp).await;
                        cb_sync.record_sequencer_drift(timestamp); // Pillar T: Track L2 Sequencer Lag
                        // Bug Fix: Wire block tracker to ensure bundle target_block is correct
                        bb_sync.block_tracker.update(block_number);
                    }
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        });
    }

    let l1_calc = L1DataFeeCalculator::new(ws_provider.clone());
    {
        let l1 = l1_calc.clone();
        let c = chain;
        tokio::spawn(async move {
            loop {
                if let Err(e) = l1.refresh_scalars(c).await {
                    error!("❌ Failed to refresh L1 scalars: {}", e);
                }
                // HIGH-SPEED: Refresh every block (2s) to track volatility accurately.
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    }

    let flash_executor = Arc::new(FlashLoanExecutor::new(
        ws_provider.clone(),
        executor_address,
        U256::from(10u64.pow(14)),
        Some(bundle_builder.clone()),
        90,
        nonce_manager.clone(),
        circuit_breaker.clone(),
        state_simulator.clone(),
        bidding_engine.clone(),
        l1_calc.clone(),
    ).await?);

    let profit_manager = Arc::new(ProfitManager::new(
        http_provider.clone(),
        wallet.clone(),
        nonce_manager.clone(),
        l1_calc.clone(),
        chain,
        U256::from_str("1000000000000000")?, // ₹200 survival threshold (0.001 ETH)
        env::var("COMPOUNDING_TARGET_ADDRESS").ok().and_then(|s| s.parse().ok())
    ));

    {
        let pm = profit_manager.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(86400));
            loop { interval.tick().await; pm.report_harvest().await; }
        });
    }

    let inventory_manager = Arc::new(InventoryManager::new(
        http_provider.clone(),
        wallet.clone(),
        executor_address,
        chain,
        circuit_breaker.clone(),
        nonce_manager.clone(),
        gas_feed.clone(),
        l1_calc.clone(), // Fix: Pass all 8 arguments to constructor
    ));

    // Pillar Z: Warm Start Discovery
    // Bug Fix: Must run warm_start before hunting to populate the pool graph
    {
        let discovery = Discovery::new(ws_provider.clone(), pool_tx.clone(), chain);
        info!("🔍 [PILLAR Z] Initializing Warm Start (Scanning historical liquidity)...");
        discovery.warm_start().await;
    }

    // Pillar Q: Bootstrap Protocol (Final Pre-flight)
    info!("🛠️ [PILLAR Q] Executing Bootstrap Protocol...");
    {
        // 1. Check Survival Budget
        inventory_manager.ensure_ready().await?;
        
        // 2. Initial State Mirror Sync
        let _ = state_mirror.sync_all_pools_multicall(http_provider.clone()).await;
        
        // 3. RPC Latency check
        let start = Instant::now();
        let _ = ws_provider.get_block_number().await?;
        let latency = start.elapsed().as_millis();
        info!("📡 [BOOTSTRAP] RPC Latency: {}ms", latency);
        if latency > 500 { warn!("⚠️ [BOOTSTRAP] High latency detected on primary RPC!"); }
    }
    info!("✅ [PILLAR Q] Bootstrap Sequence Complete. System is STABLE.");

    {
        let inv = inventory_manager.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(300));
            loop {
                interval.tick().await;
                let _ = inv.unwrap_weth_if_needed().await;
                if let Some(tokens) = constants::SAFE_TOKENS.get(&chain) {
                    let _ = inv.auto_sweep(tokens.clone()).await;
                }
            }
        });
    }

    let (mempool_listener, mut mempool_rx, _priority_rx) = MempoolListener::new(
        MempoolListenerConfig {
            endpoints: wss_urls,
            chain,
            min_gas_price_gwei: 0,
            ..Default::default()
        },
        None,
    ).await?;

    let p2p_alloy_tx = mempool_listener.p2p_tx();

    // Pillar T: Global Hydra Nervous System - gRPC Sniper Server
    let (grpc_swap_tx, mut grpc_swap_rx) = mpsc::channel(4096);
    let sniper_service = HydraNetworkImpl { tx: grpc_swap_tx };
    
    // Fast Auth: Pre-load token to avoid env lookups during hot-path
    // Robust loading: Checks for TOKEN or KEY variations
    let auth_token = env::var("SHADOW_AUTH_TOKEN").or_else(|_| env::var("ACCESS_TOKEN")).expect("🚀 SHADOW_AUTH_TOKEN required");
    let token_metadata = MetadataValue::from_str(&auth_token)?;

    let interceptor = move |req: Request<()>| {
        match req.metadata().get("x-shadow-token").or_else(|| req.metadata().get("authorization")) {
            Some(t) if t == token_metadata => Ok(req),
            _ => Err(Status::unauthenticated("Invalid Shadow Token")),
        }
    };

    // Pillar HF: Dynamic Port Discovery for Hugging Face Spaces
    let grpc_port = env::var("PORT").unwrap_or_else(|_| "50051".to_string());
    let grpc_addr = format!("0.0.0.0:{}", grpc_port).parse().expect("Invalid PORT format");

    info!("🌐 [SNIPER] gRPC Nervous System online at {} (Binary Proto-Stream)", grpc_addr);
    tokio::spawn(async move {
        let svc = HydraNetworkServer::with_interceptor(sniper_service, interceptor);
        if let Err(e) = Server::builder()
            .add_service(svc)
            .serve(grpc_addr)
            .await {
                error!("❌ [HYDRA] gRPC Server failed: {}", e);
            }
    });

    tokio::spawn(async move {
        if let Err(e) = mempool_listener.run().await {
            error!("❌ Mempool Listener crashed: {:?}", e);
        }
    });

    let (swap_tx, swap_rx) = mpsc::channel(4096);

    // Global Deduplication Cache & Shared Decoder
    let decoder = Arc::new(the_sovereign_shadow::universal_decoder::UniversalDecoder::new());
    let seen_hashes = Arc::new(DashSet::with_capacity_and_hasher(100_000, std::hash::BuildHasherDefault::<rustc_hash::FxHasher>::default()));

    // Nerve Bridge A: Connects Local Listeners to the Brain with Deduplication
    let s_tx = swap_tx.clone();
    let seen_hashes_local = seen_hashes.clone();
    let mirror_bridge = state_mirror.clone();
    tokio::spawn(async move {
        while let Some(event) = mempool_rx.recv().await {
            // Pillar U: Zero-Cost Deduplication. Skip if already captured via gRPC/P2P.
            if !seen_hashes_local.insert(event.tx_hash) {
                continue;
            }
            
            // Pillar W: Feed organic traders into the registry
            mirror_bridge.record_trader(event.swap_info.router, event.sender);

            if seen_hashes_local.len() > 100_000 { seen_hashes_local.clear(); }

            let _ = s_tx.send(event).await;
        }
    });

    // Nerve Bridge B: Connects Global Scouts to the Brain
    let s_tx_grpc = swap_tx.clone();
    let seen_hashes_grpc = seen_hashes.clone();
    let decoder_grpc = decoder.clone();
    tokio::spawn(async move {
        while let Some(raw) = grpc_swap_rx.recv().await {            
            let to = alloy_primitives::Address::from_slice(&raw.pool_address);
            let tx_hash = alloy_primitives::B256::from_slice(&raw.tx_hash);

            // Pillar U: Global Deduplication. First scout (Asia/Europe/US) to send data wins.
            if !seen_hashes_grpc.insert(tx_hash) {
                continue;
            }
            if seen_hashes_grpc.len() > 100_000 { seen_hashes_grpc.clear(); }

            // Real-time Gas Integration: Use actual gas price observed by the Scout
            let effective_gas_price = alloy_primitives::U256::from(raw.gas_price);

            let payload = bytes::Bytes::from(raw.data_payload); // Take ownership of gRPC buffer

            let decode_tx = the_sovereign_shadow::universal_decoder::DecodeTx {
                to: Some(to),
                value: alloy_primitives::U256::ZERO, 
                input: payload.clone(),
            };

            // Sniper brain decodes the raw payload filtered by Global Scouts
            let swaps = decoder_grpc.decode(&decode_tx);
            for swap in swaps {
                let is_whale = swap.amount_in > alloy_primitives::U256::from(10u128 * 10u128.pow(18)); // 10 ETH Whale
                let event = the_sovereign_shadow::mempool_listener::SwapEvent {
                    tx_hash,
                    sender: alloy_primitives::Address::ZERO, 
                    swap_info: swap,
                    effective_gas_price,
                    received_at: Instant::now(),
                    is_whale_trigger: is_whale, 
                    mempool_tx: Some(the_sovereign_shadow::models::MempoolTx {
                        data: alloy_primitives::Bytes(payload.clone()),
                        hash: tx_hash,
                        to: Some(to),
                    }),
                };
                let _ = s_tx_grpc.send(event).await;
            }
        }
    });

    the_sovereign_shadow::p2p_engine::start_p2p_bridge(p2p_alloy_tx);

    let mut detector_config = DetectorConfig::default();
    detector_config.chain = chain;
    detector_config.executor_address = executor_address;
    detector_config.bribe_percent = 60;
    detector_config.scanner_threads = 16;
    detector_config.min_profit_wei = U256::from(10u64.pow(13));

    let mut pool_rx = pool_tx.subscribe();

    // Pillar Z: Proactive Pool Initialization Task
    let mirror_init = state_mirror.clone();
    let provider_init = ws_provider.clone();
    tokio::spawn(async move {
        while let Ok(event) = pool_rx.recv().await {
            let pool_addr = match event {
                NewPoolEvent::V2(ref d) => d.pair,
                NewPoolEvent::V3(ref d) => d.pool,
            };

            // Pillar L: Proactive Bytecode Warming for X-Ray Scanning
            let m = mirror_init.clone();
            let p = provider_init.clone();
            tokio::spawn(async move {
                m.fetch_and_cache_bytecode(pool_addr, p).await;
            });

            if let NewPoolEvent::V2(data) = event {
                if data.dex_name == the_sovereign_shadow::models::DexName::Aerodrome {
                    let mirror = mirror_init.clone();
                    mirror.update_aerodrome_stable(data.pair, true);
                }
            }
        }
    });

    let pool_rx_for_detector = pool_tx.subscribe();
    let (priority_tx_dummy, priority_rx_dummy) = mpsc::channel::<the_sovereign_shadow::mempool_listener::SwapEvent>(256);
    drop(priority_tx_dummy);

    let (detector, mut opp_rx, force_tx) = ArbitrageDetector::new(
        detector_config,
        ws_provider.clone(),
        state_mirror.clone(),
        gas_feed.clone(),
        bidding_engine.clone(),
        swap_rx,
        priority_rx_dummy,
        pool_rx_for_detector,
    ).await;

    // Pillar S: Real-time State Mirror Syncing (The Lifeblood)
    {
        let mirror = state_mirror.clone();
        let provider = ws_provider.clone();
        let f_tx = force_tx.clone();

        // Pillar S: Pre-compute topics to avoid string parsing in the hot loop
        let v2_sync_topic = alloy_primitives::fixed_bytes!("0x1c411e9a96e071241c2f21f7726b17ae89e3cab4c78be50e062b03a9fffbbad1");
        let v3_swap_topic = alloy_primitives::fixed_bytes!("0xc42079f94a6350d7e6235f29174924f928cc2ac818eb64fed8004e115fbcca67");
        
        tokio::spawn(async move {
            let filter = alloy::rpc::types::Filter::new()
                .event_signature(vec![v2_sync_topic, v3_swap_topic]);

            let mut sub = provider.subscribe_logs(&filter).await.expect("Failed to sub to state logs");
            info!("📡 [STATE] Real-time Mirror Syncing active (V2 Sync & V3 Swap)");

            while let Ok(log) = sub.recv().await {
                let pool_addr = log.address();
                if !log.topics().is_empty() && log.topics()[0] == v2_sync_topic {
                    // V2 Sync Event: [reserve0, reserve1] in data
                    if log.data().data.len() >= 64 {
                        let r0 = alloy_primitives::U256::from_be_slice(&log.data().data[0..32]);
                        let r1 = alloy_primitives::U256::from_be_slice(&log.data().data[32..64]);
                        mirror.update_v2_reserves(pool_addr, r0, r1);
                    }
                } else {
                    // V3 Swap Event: [..., sqrtPriceX96, tick, ...]
                    // V3 logs are more complex, but we extract sqrtPrice from the data/topics
                    if let Some(state) = the_sovereign_shadow::v3_math::decode_v3_swap_log(&log) {
                        mirror.update_v3_state(pool_addr, state.sqrt_price, state.tick, state.liquidity);
                    }
                }
                // Trigger detector to re-evaluate cycles on state change
                let _ = f_tx.send(());
            }
        });
    }

    tokio::spawn(async move { detector.run().await; });

    info!("🚀 Sovereign Shadow LIVE — hunting alpha at nanosecond speed");
    if executor_address == Address::ZERO {
        info!("📋 DRY-RUN: deploy Executor.sol and set EXECUTOR_ADDRESS to go live");
    }

    {
        let gf = gas_feed.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                let (base, priority, _) = gf.current().await;
                info!("📊 [GAS] Base: {:.6} gwei | Priority: {:.6} gwei",
                    base.to::<u128>() as f64 / 1e9,
                    priority.to::<u128>() as f64 / 1e9);
            }
        });
    }

    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    let exec_sem = Arc::new(tokio::sync::Semaphore::new(1));
    loop {
        tokio::select! {
            Some(opp) = opp_rx.recv() => {
                if constants::GLOBAL_PAUSE.load(Ordering::Relaxed) { continue; }
                let profit_eth = opp.expected_profit.to::<u128>() as f64 / 1e18;
                if executor_address == Address::ZERO {
                    info!("💰 [DRY-RUN] Profit: {:.8} ETH | Hops: {}", profit_eth, opp.path.hops.len());
                    continue;
                }
                if let Ok(permit) = exec_sem.clone().try_acquire_owned() {
                    let exec = flash_executor.clone();
                    let pm = profit_manager.clone();
                    let tele = telemetry_handle.clone();
                    
                    tele.send(telemetry::TelemetryEvent::OpportunityFound { 
                        path: format!("{:?}", opp.path.hops), 
                        est_profit: profit_eth 
                    });

                    tokio::spawn(async move {
                        match exec.simulate_and_execute(&opp).await {
                            Ok(hash) => {
                                info!("✅ [TX] Confirmed: {:?} | {:.8} ETH", hash, profit_eth);
                                tele.send(telemetry::TelemetryEvent::ExecutionSuccess { 
                                    tx_hash: format!("{:?}", hash), 
                                    net_profit: profit_eth 
                                });
                                if let Some(d) = opp.profit_details {
                                    let pm_c = pm.clone();
                                    tokio::spawn(async move { let _ = pm_c.handle_profit(d.net_profit).await; });
                                }
                            }
                            Err(e) => {
                                error!("❌ [EXEC] Failed: {}", e);
                                tele.send(telemetry::TelemetryEvent::ExecutionFailed { error: e.to_string() });
                            }
                        }
                        drop(permit);
                    });
                }
            }
            _ = &mut shutdown => {
                info!("🛑 Shutdown — closing Sovereign Shadow");
                break;
            }
        }
    }

    Ok(())
}

async fn run_scout() -> Result<(), Box<dyn Error>> {
    dotenv().ok();
    
    let scout_id = env::var("SCOUT_ID").unwrap_or_else(|_| "global-scout-01".to_string());
    let wss_raw = env::var("SHADOW_WS_URL")
        .or_else(|_| env::var("SHADOW_WS_URL_1"))
        .expect("👁️ Scout needs SHADOW_WS_URL");
    
    let wss_urls: Vec<String> = wss_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();

    // Default to dynamic port if not specified (Hugging Face default)
    let sniper_url = env::var("SNIPER_URL").unwrap_or_else(|_| "http://0.0.0.0:7860".to_string());
    
    info!("👁️ [SCOUT {}] Initializing Global Eye", scout_id);

    // 1. Setup gRPC Connection to Space B (Sniper)
    let channel = tonic::transport::Channel::from_shared(sniper_url)?
        .tcp_nodelay(true) 
        .connect_lazy();
    let mut client = hydra::hydra_network_client::HydraNetworkClient::new(channel);

    let (grpc_tx, grpc_rx) = mpsc::channel(4096);
    let stream = ReceiverStream::new(grpc_rx);

    tokio::spawn(async move {
        if let Err(e) = client.stream_opportunities(stream).await {
            error!("❌ [SCOUT] gRPC Stream broken: {}", e);
        }
    });

    // 2. Setup Local Mempool Listener
    let chain_name = env::var("CHAIN").unwrap_or_else(|_| "base".to_string());
    let chain = match chain_name.as_str() {
        "base" => the_sovereign_shadow::models::Chain::Base,
        _ => the_sovereign_shadow::models::Chain::Mainnet,
    };

    let (mempool_listener, mut mempool_rx, _) = MempoolListener::new(
        MempoolListenerConfig {
            endpoints: wss_urls,
            chain,
            ..Default::default()
        },
        None,
    ).await?;

    tokio::spawn(async move {
        if let Err(e) = mempool_listener.run().await {
            error!("❌ Scout Mempool Listener crashed: {:?}", e);
        }
    });

    info!("🚀 [SCOUT {}] Space A ACTIVE - Gossiping binary data to Sniper", scout_id);

    while let Some(event) = mempool_rx.recv().await {
        let opportunity = RawOpportunity {
            tx_hash: event.tx_hash.to_vec(),
            pool_address: event.swap_info.router.to_vec(),
            data_payload: event.mempool_tx.map(|m| m.data.0.to_vec()).unwrap_or_default(),
            timestamp: event.received_at.elapsed().as_nanos() as i64,
            gas_price: event.effective_gas_price.to::<u64>(),
        };

        let _ = grpc_tx.try_send(opportunity);
    }

    Ok(())
}

struct HydraNetworkImpl {
    tx: mpsc::Sender<RawOpportunity>,
}

#[tonic::async_trait]
impl HydraNetwork for HydraNetworkImpl {
    type StreamOpportunitiesStream = ReceiverStream<Result<Empty, Status>>;

    async fn stream_opportunities(
        &self,
        request: Request<Streaming<RawOpportunity>>,
    ) -> Result<Response<ReceiverStream<Result<Empty, Status>>>, Status> {
        let mut stream = request.into_inner();
        let tx = self.tx.clone();
        
        // Persistent HTTP/2 stream handler for Global Scouts
        tokio::spawn(async move {
            while let Some(opportunity) = stream.next().await {
                match opportunity {
                    Ok(opp) => {
                        if let Err(e) = tx.send(opp).await {
                            error!("❌ [HYDRA] Internal bridge broken: {}", e);
                            break;
                        }
                    }
                    Err(e) => {
                        warn!("⚠️ [HYDRA] Scout connection dropped: {}", e);
                        break;
                    }
                }
            }
        });
        let (_, rx) = mpsc::channel::<Result<Empty, Status>>(1);
        Ok(Response::new(ReceiverStream::new(rx)))
    }
}
