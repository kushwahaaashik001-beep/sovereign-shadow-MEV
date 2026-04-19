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
use the_sovereign_shadow::mempool_listener::MempoolListener;
use the_sovereign_shadow::utils::{CircuitBreaker, L1DataFeeCalculator};
use the_sovereign_shadow::{constants, telemetry, WsProviderPool};
use dotenv::dotenv;
use alloy::providers::{ProviderBuilder, WsConnect, Provider};
use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, U256, fixed_bytes};
use std::error::Error;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use futures_util::StreamExt;
use std::env;
use tracing::{error, info, warn};
use dashmap::DashSet;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, filter::LevelFilter};

/// Robustly parses an address from a string, handling optional '0x' prefix.
fn parse_address_robust(s: &str) -> Address {
    let s = s.trim();
    if s.is_empty() { return Address::ZERO; }
    
    if s.starts_with("0x") {
        s.parse().unwrap_or(Address::ZERO)
    } else {
        format!("0x{}", s).parse().unwrap_or(Address::ZERO)
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    // Dummy Server for Hugging Face Health Check
    // Hugging Face expects a web server on port 7860 to keep the Space alive.
    std::thread::spawn(|| {
        let listener = std::net::TcpListener::bind("0.0.0.0:7860").unwrap();
        println!("📢 Dummy Web Server started on port 7860 for Hugging Face");
        for stream in listener.incoming() {
            if let Ok(mut stream) = stream {
                let response = "HTTP/1.1 200 OK\r\n\r\nSovereign Shadow is LIVE!";
                let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
            }
        }
    });

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

    info!("🎯 Sovereign Shadow Unified Engine Starting...");
    runtime.block_on(run_engine())
}

async fn run_engine() -> Result<(), Box<dyn Error>> {
    dotenv().ok();

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env().add_directive(LevelFilter::INFO.into()))
        .init();

    info!("🛸 Sovereign Shadow MEV Engine — BEAST MODE ONLINE");

    let chain = the_sovereign_shadow::models::Chain::Base;
    info!("⛓️  Chain: {:?}", chain);

    // Load WebSocket endpoints for mempool streaming
    let wss_raw = env::var("SHADOW_WS_URL")
        .or_else(|_| env::var("SHADOW_WS_URL_1"))
        .expect("🚀 Sniper needs SHADOW_WS_URL or SHADOW_WS_URL_1");

    let wss_urls: Vec<String> = wss_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();

    // Load HTTP RPC for state reads (Alchemy/Quicknode)
    let rpc_raw = env::var("SHADOW_RPC_URL")
        .or_else(|_| env::var("SHADOW_RPC_URL_1"))
        .expect("🚀 Sniper needs SHADOW_RPC_URL or SHADOW_RPC_URL_1");

    let http_urls: Vec<String> = rpc_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();

    let priv_key_raw = env::var("PRIVATE_KEY").expect("🚀 Needs PRIVATE_KEY for execution");
    let priv_key = priv_key_raw.trim().trim_start_matches("0x");

    let relay_key_raw = env::var("RELAY_SIGNING_KEY").expect("🚀 Needs RELAY_SIGNING_KEY for MEV-Blocker identity");
    let relay_key = relay_key_raw.trim().trim_start_matches("0x");

    // [GOD-LEVEL] Provider Filtering: Sirf wahi keys use hongi jo actually kaam kar rahi hain.
    let mut working_wss = Vec::new();
    let mut working_ws_providers = Vec::new();
    for url in wss_urls {
        let ws = WsConnect::new(url.clone());
        if let Ok(p) = tokio::time::timeout(Duration::from_secs(5), ProviderBuilder::new().on_ws(ws)).await {
            if let Ok(prov) = p {
                working_ws_providers.push(Arc::new(prov.boxed()));
                working_wss.push(url);
            }
        }
    }

    if working_ws_providers.is_empty() { return Err("No valid WSS endpoints found".into()); }
    let ws_provider_pool = Arc::new(WsProviderPool::new(working_ws_providers));
    
    let mut working_http = Vec::new();
    let mut working_http_providers = Vec::new();
    for url in http_urls {
        if let Ok(parsed_url) = url.parse() {
            let prov = ProviderBuilder::new().on_http(parsed_url);
            working_http_providers.push(Arc::new(prov.boxed()));
            working_http.push(url);
        }
    }
    if working_http_providers.is_empty() { return Err("No valid HTTP endpoints found".into()); }
    let http_provider_pool = Arc::new(WsProviderPool::new(working_http_providers));

    // Initialize Telemetry Nervous System
    let (tele_tx, tele_rx) = mpsc::unbounded_channel();
    let telemetry_handle = Arc::new(telemetry::TelemetryHandle::new(tele_tx));
    let tele_handle_for_loop = telemetry_handle.clone();
    tokio::spawn(async move {
        telemetry::run_telemetry_loop(tele_rx).await;
    });
    
    // Hydra Assignment: Head 0 for Setup and Heartbeat
    let (_, ws_setup_provider) = ws_provider_pool.get_head(0);
    
    let executor_address = parse_address_robust(&env::var("EXECUTOR_ADDRESS").unwrap_or_default());

    let chain_id = ws_setup_provider.get_chain_id().await?;
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

    let nonce_manager = Arc::new(NonceManager::new(ws_setup_provider.clone(), wallet.address()).await?);

    // Pillar E: Dynamic Relay Loading from Secrets
    let mut relays = vec![
        "https://relay-base.flashbots.net".to_string(),
        "https://base.mevblocker.io".to_string(),
    ];
    if let Ok(custom_relay) = env::var("PRIVATE_RELAY_URL") {
        relays.push(custom_relay);
    }

    let l2_rpcs: Vec<String> = env::var("PRIVATE_RPCS")
        .unwrap_or_default().split(',')
        .filter(|s| !s.is_empty()).map(String::from).collect();

    let identity_wallet = PrivateKeySigner::from_str(relay_key)?;

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
        ws_setup_provider.clone(),
        nonce_manager.clone(),
        circuit_breaker.clone(),
        state_simulator.clone(),
    ).await?);

    // Pillar S: Self-Healing Memory Management (8GB RAM Safety)
    {
        let mirror = state_mirror.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop { interval.tick().await; mirror.prune_stale_pools(600); }
        });

        // [SILENT SNIPER] Periodic Bytecode Persistence (Save to disk every 1 hour)
        let mirror_persist = state_mirror.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3600));
            loop {
                interval.tick().await;
                mirror_persist.save_bytecode_cache();
            }
        });

        // [SILENT SNIPER] State Checkpoint (Save to disk every 15 minutes)
        let mirror_state_persist = state_mirror.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(900));
            loop {
                interval.tick().await;
                mirror_state_persist.save_state_checkpoint();
            }
        });
    }

    let l1_calc = L1DataFeeCalculator::new(ws_setup_provider.clone());

    let flash_executor = Arc::new(FlashLoanExecutor::new(
        ws_setup_provider.clone(),
        executor_address,
        U256::from(10u64.pow(14)),
        Some(bundle_builder.clone()),
        90,
        nonce_manager.clone(),
        circuit_breaker.clone(),
        state_simulator.clone(),
        bidding_engine.clone(),
        l1_calc.clone(),
        Some(http_provider_pool.clone()),
    ).await?);

    // [CLEANUP] Redundant block tracker removed. Logic is handled by Unified Heartbeat below.

    let profit_manager = Arc::new(ProfitManager::new(
        http_provider_pool.get_head(2).1, // Role: HTTP_FLASHBOTS (Head 2)
        wallet.clone(),
        nonce_manager.clone(),
        l1_calc.clone(),
        chain,
        U256::from(40_000_000_000_000_000u128), // $100 survival threshold (~0.04 ETH)
        env::var("COMPOUNDING_TARGET_ADDRESS").ok().map(|s| parse_address_robust(&s))
            .or(constants::GAS_VAULT_ADDRESS) // Fallback to secondary address from constants.rs
    ));

    {
        let pm = profit_manager.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(86400));
            loop { interval.tick().await; pm.report_harvest().await; }
        });
    }

    let inventory_manager = Arc::new(InventoryManager::new(
        http_provider_pool.get_head(3).1, // Role: HTTP_BACKUP (Head 3)
        wallet.clone(),
        executor_address,
        chain,
        circuit_breaker.clone(),
        nonce_manager.clone(),
        gas_feed.clone(),
        l1_calc.clone(), // Fix: Pass all 8 arguments to constructor
    ));

    // Nerve Bridge C: Pre-initialize Detector to catch Bootstrap events
    let (priority_tx_dummy, priority_rx_dummy) = mpsc::channel::<the_sovereign_shadow::mempool_listener::SwapEvent>(256);
    drop(priority_tx_dummy);

    let (swap_tx, swap_rx) = mpsc::channel(4096);
    
    let mut detector_config = DetectorConfig::default();
    detector_config.chain = chain;
    detector_config.executor_address = executor_address;
    detector_config.bribe_percent = 60;
    detector_config.scanner_threads = 16;
    detector_config.min_profit_wei = U256::from(2 * 10u128.pow(14)); // $0.50 floor for stability

    let (detector, mut opp_rx, _) = ArbitrageDetector::new(
        detector_config,
        ws_setup_provider.clone(),
        state_mirror.clone(),
        gas_feed.clone(),
        bidding_engine.clone(),
        swap_rx,
        priority_rx_dummy,
        pool_tx.subscribe(),
    ).await;

    // Nerve Bridge C: Start the Brain immediately so it catches the Bootstrap events
    tokio::spawn(async move { detector.run().await; });

    // Point #1: Full Sync happens ONLY at bootstrap. 
    info!("🛠️ [PILLAR Q] Executing Bootstrap Protocol...");
    {
        // Point #2: Background "Lazy" Bootstrapping
        let inv = inventory_manager.clone();
        let mirror = state_mirror.clone();
        let pool = http_provider_pool.clone();
        
        let _ = tokio::try_join!(
            inv.ensure_ready(),
            mirror.smart_bootstrap(pool)
        );

        let _ = ws_setup_provider.get_block_number().await?;
    }
    info!("✅ [PILLAR Q] Bootstrap Sequence Complete. System is STABLE.");

    {
        let inv = inventory_manager.clone();
        tokio::spawn(async move {
            let mut gas_check_interval = tokio::time::interval(Duration::from_secs(300)); // 5 mins for Gas refill
            let mut harvest_interval = tokio::time::interval(Duration::from_secs(3600)); // 1 hour for Profit Harvest

            loop {
                tokio::select! {
                    _ = gas_check_interval.tick() => {
                        let _ = inv.unwrap_weth_if_needed().await;
                    }
                    _ = harvest_interval.tick() => {
                        info!("🌾 [PILLAR J] Starting Hourly Profit Harvest...");
                        let _ = inv.auto_sweep().await;
                    }
                }
            }
        });
    }

    // [GOD-LEVEL] Unified Heartbeat: Sirf 1 WSS Connection poore bot ke liye
    let (unified_log_tx, mut unified_log_rx) = mpsc::unbounded_channel();
    let (unified_block_tx, _) = broadcast::channel(128);
    
    let wss_heartbeat_url = working_wss[0].clone(); // Use only the most stable key
    let mirror_heartbeat = state_mirror.clone();
    let block_tx_heartbeat = unified_block_tx.clone();
    let bb_heartbeat = bundle_builder.clone();
    let cb_heartbeat = circuit_breaker.clone();

    tokio::spawn(async move {
        loop {
            let ws = WsConnect::new(wss_heartbeat_url.clone());
            if let Ok(provider) = ProviderBuilder::new().on_ws(ws).await {
                info!("✅ [HEARTBEAT] Unified Pipe Connected: {}", wss_heartbeat_url);
                
                let v2_sync = fixed_bytes!("1c411e9a96e071241c2f21f7726b17ae89e3cab4c78be50e062b03a9fffbbad1");
                let v3_swap = fixed_bytes!("c42079f94a6350d7e6235f29174924f928cc2ac818eb64fed8004e115fbcca67");
                
                // PILLAR Z: Global Subscription (No address filter)
                // RPC se sirf topic mango, filtering Rust side par HashSet se hogi.
                let filter = alloy::rpc::types::Filter::new().event_signature(vec![v2_sync, v3_swap]);

                let sub_logs = provider.subscribe_logs(&filter).await;
                let sub_blocks = provider.subscribe_blocks().await;

                if let (Ok(logs), Ok(blocks)) = (sub_logs, sub_blocks) {
                    let mut log_stream = logs.into_stream();
                    let mut block_stream = blocks.into_stream();

                    loop {
                        tokio::select! {
                            Some(log) = log_stream.next() => {
                                // PILLAR S: Local Filter (Zero-Cost)
                                // Sirf un pools ko update karo jo registry mein hain.
                                MempoolListener::update_mirror_state(&log, &mirror_heartbeat);
                                let _ = unified_log_tx.send(log);
                            }
                            Some(block) = block_stream.next() => {
                                let bn = block.header.number;
                                let fee = U256::from(block.header.base_fee_per_gas.unwrap_or_default());
                                mirror_heartbeat.sync_block(bn, fee, block.header.timestamp).await;
                                cb_heartbeat.record_sequencer_drift(block.header.timestamp);
                                bb_heartbeat.block_tracker.update(bn);
                                let _ = block_tx_heartbeat.send(bn);
                            }
                            else => break,
                        }
                    }
                }
            }
            warn!("⚠️ [HEARTBEAT] Connection lost. Reconnecting in 5s...");
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });

    // Global Deduplication Cache & Shared Decoder
    let _decoder = Arc::new(the_sovereign_shadow::universal_decoder::UniversalDecoder::new());
    let seen_hashes = Arc::new(DashSet::with_capacity_and_hasher(50_000, std::hash::BuildHasherDefault::<rustc_hash::FxHasher>::default()));

    // Nerve Bridge A: Connects Local Listeners to the Brain with Deduplication
    let s_tx = swap_tx.clone();
    let seen_hashes_local = seen_hashes.clone();
    let mirror_bridge = state_mirror.clone();
    tokio::spawn(async move {
        while let Some(log) = unified_log_rx.recv().await {
            // Pillar U: Zero-Cost Deduplication. Multi-endpoint streaming sync.
            let tx_hash = log.transaction_hash.unwrap_or_default();
            if !seen_hashes_local.insert(tx_hash) { continue; }

            // PILLAR Z: Convert Global Log to SwapEvent for the Brain
            if mirror_bridge.pools.contains_key(&log.address()) {
                let event = the_sovereign_shadow::mempool_listener::SwapEvent {
                    tx_hash,
                    sender: alloy_primitives::Address::ZERO,
                    swap_info: the_sovereign_shadow::models::SwapInfo {
                        dex: the_sovereign_shadow::models::DexName::UniswapV2,
                        router: log.address(),
                        ..Default::default()
                    },
                    effective_gas_price: alloy_primitives::U256::ZERO,
                    received_at: std::time::Instant::now(),
                    is_whale_trigger: false,
                    mempool_tx: None,
                };
                let _ = s_tx.send(event).await;
            }
        }
    });

    let mut pool_rx = pool_tx.subscribe();

    // Pillar Z: Proactive Pool Initialization Task
    let mirror_init = state_mirror.clone();
    let http_pool_init = http_provider_pool.clone();
    tokio::spawn(async move {
        while let Ok(event) = pool_rx.recv().await {
                let (pool_addr, dex_type) = match event {
                    NewPoolEvent::V2(ref d) => {
                        let dt = match d.dex_name {
                            the_sovereign_shadow::models::DexName::Aerodrome => the_sovereign_shadow::models::DexType::Aerodrome,
                            _ => the_sovereign_shadow::models::DexType::UniswapV2,
                        };
                        (d.pair, dt)
                    }
                    NewPoolEvent::V3(ref d) => (d.pool, the_sovereign_shadow::models::DexType::UniswapV3),
            };

                // Seed state mirror so background sync can pick it up
                mirror_init.pools.entry(pool_addr).or_insert(the_sovereign_shadow::state_mirror::PoolState {
                    dex_type,
                    ..Default::default()
                });

            // Pillar L: Proactive Bytecode Warming for X-Ray Scanning
            // HTTP pool ka use karke WebSocket connections aur rate limits bacha rahe hain.
            let m = mirror_init.clone();
            let p = http_pool_init.get_head(1).1; // Role: HTTP_SIMULATE (Head 1)
            tokio::spawn(async move {
                m.fetch_and_cache_bytecode(pool_addr, p).await;
            });

                if let NewPoolEvent::V2(ref data) = event {
                if data.dex_name == the_sovereign_shadow::models::DexName::Aerodrome {
                    let mirror = mirror_init.clone();
                    mirror.update_aerodrome_stable(data.pair, true);
                }
            }
        }
    });

    // Pillar Z: Warm Start Discovery
    // Bug Fix: Must run warm_start before hunting to populate the pool graph
    {
        let discovery = Discovery::new(http_provider_pool.clone(), pool_tx.clone(), chain); // Use HTTP_SYNC (Head 0) for discovery
        info!("🔍 [PILLAR Z] Initializing Warm Start (Scanning historical liquidity)...");
        discovery.bootstrap_core_pools(); // Seeding happens while listeners are active
        discovery.warm_start().await;
    }

    // [REDUNDANT] Task merged into Unified Heartbeat to save RPC connections.

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
                    let inv_c = inventory_manager.clone();
                    
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

                                // Register tokens for future harvesting
                                for hop in &opp.path.hops {
                                    inv_c.register_token_for_harvest(hop.token_in);
                                    inv_c.register_token_for_harvest(hop.token_out);
                                }

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
                state_mirror.save_bytecode_cache(); // Final save before exit
                break;
            }
        }
    }

    Ok(())
}
