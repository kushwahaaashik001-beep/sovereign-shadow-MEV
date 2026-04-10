#![allow(dead_code)]
#![allow(unused_variables)]

use the_sovereign_shadow::Opportunity;

use the_sovereign_shadow::arbitrage_detector::{ArbitrageDetector, DetectorConfig};
use the_sovereign_shadow::bundle_builder::{BundleBuilder, BundleBuilderConfig};
use the_sovereign_shadow::flash_loan_executor::FlashLoanExecutor;
use the_sovereign_shadow::gas_feed::GasPriceFeed;
use the_sovereign_shadow::inventory_manager::InventoryManager;
use the_sovereign_shadow::nonce_manager::NonceManager;
use the_sovereign_shadow::profit_manager::ProfitManager;
use the_sovereign_shadow::state_mirror::StateMirror;
use the_sovereign_shadow::factory_scanner::{FactoryScanner, NewPoolEvent};
use the_sovereign_shadow::models::DexName;
use the_sovereign_shadow::discovery;
use the_sovereign_shadow::state_simulator::StateSimulator; // Import StateSimulator
use the_sovereign_shadow::bidding_engine::BiddingEngine;
use the_sovereign_shadow::mempool_listener::{MempoolListener, MempoolListenerConfig};
use the_sovereign_shadow::utils::{CircuitBreaker, L1DataFeeCalculator, audit_log, cleanup_auditor_logs};
use the_sovereign_shadow::constants;
use the_sovereign_shadow::rpc_manager::RpcManager;
use dotenv::dotenv;
use ethers::prelude::*;
use ethers::providers::{Provider, Ws};
use ethers::signers::{LocalWallet, Signer};
use ethers::types::BlockNumber;
use std::error::Error;
use std::str::FromStr; // Import FromStr
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use std::env;
use tracing::{debug, error, info, warn};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, filter::LevelFilter};
use tokio::io::AsyncWriteExt; // Keep this, it's used for the HF health check

use the_sovereign_shadow::WsProviderPool;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Pillar A: Configuration Authority
    // Hugging Face handles secrets via Env Vars, so we don't panic if .env is missing.
    let _ = dotenv().ok();

    // --- 📡 PILLAR HF: HEALTH-CHECK SERVER ---
    // Keeps Hugging Face Space active by listening on the required port.
    let hf_port = env::var("PORT").unwrap_or_else(|_| "7860".to_string());
    let public_url = env::var("SHADOW_PUBLIC_URL").ok(); // Set this in HF Secrets: e.g., https://your-name-space.hf.space
    
    tokio::spawn(async move {
        if let Ok(listener) = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", hf_port)).await {
            info!("📡 [PILLAR HF] Health-Check Dashboard online at port {}", hf_port);
            while let Ok((mut stream, _)) = listener.accept().await {
                let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
                                <html><body style='font-family:sans-serif; background:#111; color:#0f0; padding:20px;'>\
                                <h1>🛸 Sovereign Shadow MEV Engine</h1>\
                                <p>Status: <b>ACTIVE</b></p>\
                                <p>Mode: Hunting Alpha</p>\
                                <p>Heartbeat: OK</p>\
                                </body></html>";
                let _ = stream.write_all(response.as_bytes()).await;
            }
        }
    });

    // --- 🛡️ PILLAR HF: SELF-WAKEUP LOOP ---
    // Periodically pings itself to prevent Hugging Face from sleeping after 48h.
    if let Some(url) = public_url {
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            let mut interval = tokio::time::interval(Duration::from_secs(600)); // Every 10 mins
            loop {
                interval.tick().await;
                match client.get(&url).send().await {
                    Ok(_) => debug!("💓 [PILLAR HF] Self-ping successful. Staying awake."),
                    Err(e) => warn!("⚠️ [PILLAR HF] Self-ping failed: {}. Check SHADOW_PUBLIC_URL.", e),
                }
            }
        });
    } else {
        warn!("⚠️ [PILLAR HF] SHADOW_PUBLIC_URL not set. Bot might go to sleep mode.");
    }

    // --- 🔍 SHADOW DIAGNOSTICS ---
    println!("--- 🔍 SHADOW DIAGNOSTICS ---");
    for (key, value) in std::env::vars() {
        if key.starts_with("SHADOW_") || key.starts_with("RPC_") {
            println!("{}: {}", key, value);
        }
    }
    println!("----------------------------");

    // [STRICT AUTHORITY] Force load critical variables BEFORE any logic starts
    let priv_key_raw = env::var("SHADOW_PRIVATE_KEY")
        .expect("FATAL: SHADOW_PRIVATE_KEY missing from .env. Execution authority is required.");
    
    let manual_v2_factory: Option<Address> = env::var("SHADOW_V2_FACTORY")
        .ok()
        .and_then(|s| s.parse().ok());

    let manual_aero_factory: Option<Address> = env::var("SHADOW_AERO_FACTORY")
        .or_else(|_| env::var("AERODROME_FACTORY"))
        .ok()
        .and_then(|s| s.parse().ok());

    info!("📂 Working Directory: {:?}", env::current_dir()?);

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(
            EnvFilter::from_default_env()
                .add_directive(LevelFilter::INFO.into()) // Change to LevelFilter::TRACE to see detailed arbitrage detection logs
                // Silence noisy ethers transport reconnect spam
                .add_directive("ethers_providers::rpc::transports::ws=warn".parse().unwrap()),
        )
        .init();

    info!("🛸 Sovereign Shadow MEV Engine — INITIALIZING BEAST MODE");

    // ── Chain config ──────────────────────────────────────────────────────────
    let chain_name = env::var("SHADOW_CHAIN")
        .or_else(|_| env::var("CHAIN"))
        .unwrap_or_else(|_| "base".to_string());
    let chain = match chain_name.as_str() {
        "base"     => Chain::Base,
        "arbitrum" => Chain::Arbitrum,
        _          => Chain::Mainnet,
    };

    // Pillar A: Intelligent Provider Discovery
    // Scans for individual secrets (RPC_HTTPS_1, RPC_HTTPS_2, etc.) to maximize rate limits on HF.
    let http_urls = {
        let mut urls = Vec::new();
        for i in 1..=10 {
            if let Ok(u) = env::var(format!("RPC_HTTPS_{}", i)) {
                let u = u.trim().to_string();
                if !u.is_empty() && !urls.contains(&u) { urls.push(u); }
            }
        }
        let raw = env::var("SHADOW_RPC_URLS").or_else(|_| env::var("SHADOW_RPC_URL")).unwrap_or_default();
        for u in raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
            if !urls.contains(&u) { urls.push(u); }
        }
        if urls.is_empty() {
            return Err(Box::<dyn Error>::from("FATAL: No HTTP providers found. Check Hugging Face Secrets (RPC_HTTPS_1..4)."));
        }
        urls
    };

    let wss_urls = {
        let mut urls = Vec::new();
        for i in 1..=10 {
            if let Ok(u) = env::var(format!("RPC_WSS_{}", i)) {
                let u = u.trim().to_string();
                if !u.is_empty() && !urls.contains(&u) { urls.push(u); }
            }
        }
        let raw = env::var("SHADOW_WS_URLS").or_else(|_| env::var("SHADOW_WS_URL")).unwrap_or_default();
        for u in raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
            if !urls.contains(&u) { urls.push(u); }
        }
        if urls.is_empty() {
            return Err(Box::<dyn Error>::from("FATAL: No WSS providers found. Check Hugging Face Secrets (RPC_WSS_1..4)."));
        }
        urls
    };

    // [DYNAMIC SCALING] Automatically optimize batch size based on unique provider count
    let batch_size: usize = env::var("SHADOW_BATCH_SIZE")
        .ok().and_then(|v| v.parse().ok())
        .unwrap_or(if http_urls.len() > 1 { http_urls.len() * 3 } else { 4 });

    // [PRODUCTION] Bootstrap limit for discovery
    let bootstrap_count: usize = env::var("SHADOW_POOL_BOOTSTRAP_COUNT")
        .ok().and_then(|v| v.parse().ok())
        .unwrap_or(3500); 
    
    info!("🛠️ [CONSTRAINTS] Nodes: HTTP:{} / WSS:{} | Batch: {} | Bootstrap: {}", 
        http_urls.len(), wss_urls.len(), batch_size, bootstrap_count);

    let priv_key     = priv_key_raw.trim().trim_start_matches("0x");

    // [FORCE LOAD] Strictly use SHADOW_EXECUTOR_ADDRESS from .env
    let executor_address: Address = env::var("SHADOW_EXECUTOR_ADDRESS")
        .expect("FATAL: SHADOW_EXECUTOR_ADDRESS missing from .env. Deploy contract first.")
        .parse()
        .expect("FATAL: Invalid SHADOW_EXECUTOR_ADDRESS hex format");
    
    // ── Providers with Failover (Phase B) ─────────────────────────────────────
    let mut ws_provider_option: Option<Provider<Ws>> = None;
    for url in &wss_urls {
        match Provider::<Ws>::connect(url).await {
            Ok(p) => {
                info!("✅ Connected to WSS provider: {}", url);
                ws_provider_option = Some(p);
                break;
            }
            Err(e) => {
                warn!("⚠️ Failed to connect to WSS provider {}: {:?}", url, e);
            }
        }
    }

    let ws_provider = ws_provider_option.ok_or_else(|| {
        error!("FATAL: Could not connect to any WSS provider. Check SHADOW_WS_URLS in .env");
        Box::<dyn Error>::from("No WSS provider available")
    })?;
    let ws_provider = Arc::new(ws_provider);

    // Create a pool of WSS providers for block listener and factory scanner
    let mut ws_providers_for_pool = Vec::new();
    for url in &wss_urls {
        if let Ok(p) = Provider::<Ws>::connect(url).await {
            ws_providers_for_pool.push(Arc::new(p));
        }
    }
    let ws_provider_pool = Arc::new(WsProviderPool::new(ws_providers_for_pool));

    // ── HTTP Provider Manager (Round-Robin for Rate Limit Avoidance) ──────────
    let http_rpc_manager = Arc::new(RpcManager::new(http_urls));
    
    let chain_id = ws_provider.get_chainid().await?.as_u64();

    // ── Wallet ────────────────────────────────────────────────────────────────
    let wallet = LocalWallet::from_str(priv_key).map_err(|e| { // Use `priv_key` directly
        error!("FATAL: Private Key parsing failed. Check SHADOW_PRIVATE_KEY format in .env");
        e
    })?.with_chain_id(chain_id);
    
    info!("👛 MEV Wallet Address: {:?}", wallet.address());

    // ── Core infrastructure ───────────────────────────────────────────────────
    // [PILLAR V] Tightened Circuit Breaker for ₹200 Budget
    // Max 3 failures instead of 5, and 60s cooldown to analyze "Why it failed".
    let circuit_breaker = Arc::new(CircuitBreaker::new(3, 60));
    let l1_fee_calc     = Arc::new(L1DataFeeCalculator::new(http_rpc_manager.get_next_provider())); // This still uses HTTP provider
    let gas_feed        = Arc::new(GasPriceFeed::new(ws_provider_pool.clone(), chain).await); // [FIX] Use WsProviderPool here

    // Multicall3 — same address on all chains
    let multicall3: Address = "0xcA11bde05977b3631167028862bE2a173976CA11".parse().expect("Invalid Multicall3 address");

    // ── Pillar B: State Mirror ────────────────────────────────────────────────
    let state_mirror = StateMirror::new(ws_provider.clone(), http_rpc_manager.clone(), multicall3);

    // ── Bidding Engine (Pillar I) ──
    let bidding_engine = Arc::new(BiddingEngine::new(state_mirror.clone()));


    // ── Pillar Z: Factory Scanner ─────────────────────────────────────────────
    let (pool_tx, _) = broadcast::channel::<NewPoolEvent>(2048);
    {
        let scanner = Arc::new(FactoryScanner::new(ws_provider_pool.clone(), pool_tx.clone(), chain)); // Use WsProviderPool and pass chain
        tokio::spawn(async move { scanner.run().await; });
    }

    // ── Nonce Managers ────────────────────────────────────────────────────────
    let nonce_manager      = Arc::new(NonceManager::new(ws_provider.clone(), wallet.address()).await?);
    let nonce_manager_http = Arc::new(NonceManager::new(http_rpc_manager.get_next_provider(), wallet.address()).await?);

    // ── Pillar E: Bundle Builder ──────────────────────────────────────────────
    let relays = vec![
        "https://relay.flashbots.net".to_string(),
        "https://rpc.beaverbuild.org/".to_string(),
        "https://rpc.titanbuilder.xyz/".to_string(),
    ];

    let l2_rpcs: Vec<String> = env::var("PRIVATE_RPCS")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();

    let identity_key_raw = env::var("FLASHBOTS_IDENTITY_KEY").unwrap_or_else(|_| priv_key_raw.clone());
    let identity_key     = identity_key_raw.trim().trim_start_matches("0x");
    let identity_wallet  = LocalWallet::from_str(identity_key)?;

    let state_simulator = Arc::new(StateSimulator::new(state_mirror.clone(), executor_address));

    let bundle_builder = Arc::new(BundleBuilder::new(
        BundleBuilderConfig {
            chain_id,
            chain,
            signer:                   wallet.clone(),
            identity_signer:          identity_wallet,
            executor_address,
            min_profit_eth:           U256::from(10u64.pow(12)), // 0.000001 ETH (₹2-3 profit is enough to start scaling)
            relays,
            l2_private_rpcs:          l2_rpcs.clone(),
            base_bribe_percent:       50, // Pillar I: Survival Mode Bribe (Greedy Miner Filter)
            max_gas_price_gwei:       500,
            enable_simulation:        true,
            use_flashbots_simulation: chain == Chain::Mainnet,
            check_flash_loan:         false,
            relay_timeout_ms:         500,
            stealth_jitter:           true,
            use_raw_encoding:         false,
            nonce_recovery_blocks:    3,
            max_consecutive_failures: 5,
            pause_duration_secs:      30,
            ai_strategy:              None,
            telemetry_tx:             None,
        },
        http_rpc_manager.get_next_provider(),
        nonce_manager_http,
        circuit_breaker.clone(),
            state_simulator.clone(),
    ).await?);

    // ── Pillar M: Flash Loan Executor ─────────────────────────────────────────
    let flash_executor = Arc::new(FlashLoanExecutor::new(
        ws_provider.clone(),
        http_rpc_manager.get_next_provider(),
        wallet.clone(), // This wallet is used for signing, not for RPC.
        executor_address,
        l2_rpcs,
        U256::from(10u64.pow(12)),       // 0.000001 ETH min profit
        chain == Chain::Mainnet,
        Some(bundle_builder.clone()),
        50,                              // Pillar I: Keep 50% profit for the Vault
        U256::from(1_000_000_000u64),    // 1 gwei base priority
        U256::from(500_000_000_000u64),  // 500 gwei max gas
        nonce_manager,
        circuit_breaker.clone(),
        l1_fee_calc.clone(),
        state_simulator,
        bidding_engine.clone(),
    ).await?);

    // ── Pillar P: Profit Manager ──────────────────────────────────────────────
    let profit_manager = Arc::new(ProfitManager::new(
        ws_provider.clone(),
        wallet.clone(),
        U256::from_dec_str("11500000000000000")?, // 💰 PILLAR P: Target $30 threshold for Cold Vault harvest
        env::var("COMPOUNDING_TARGET_ADDRESS").ok().and_then(|s| s.parse().ok()),
    ));

    // ── Pillar J: Inventory Manager ───────────────────────────────────────────
    let inventory_manager = Arc::new(InventoryManager::new(
        ws_provider.clone(),
        wallet.clone(),
        l1_fee_calc.clone(),
        chain,
    ));

    {
        let im       = inventory_manager.clone();
        let provider = ws_provider.clone();
        let addr     = wallet.address();
        let cb       = circuit_breaker.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3600));
            loop {
                interval.tick().await;
                if let Ok(bal) = provider.get_balance(addr, None).await {
                    cb.update_balance(bal);
                    let eth = bal.as_u128() as f64 / 1e18;
                    if eth < 0.001 {
                        warn!("⚠️ [WALLET] LOW BALANCE: {:.6} ETH — bot may not execute!", eth);
                        the_sovereign_shadow::utils::send_telegram_msg(&format!("⚠️ *LOW BALANCE ALERT!* ⚠️\n\nWallet balance: `{:.6} ETH`\nPlease top up to avoid execution halts.", eth));
                    } else {
                        info!("[Pillar J] Wallet: {:.6} ETH", eth);
                    }
                }

                // Pillar J: Autonomous Dust Sweep
                if let Some(tokens) = constants::SAFE_TOKENS.get(&chain) {
                    let _ = im.auto_sweep(tokens.clone()).await;
                }

                // [PILLAR J] Unstoppable Scaling: Convert WETH profits to ETH gas if needed
                if let Err(e) = im.unwrap_weth_if_needed().await {
                    warn!("⚠️ [BOOTSTRAP] WETH Unwrap failed: {:?}", e);
                }
            }
        });
    }

    // ── Pillar A: Mempool Listener ────────────────────────────────────────────
    // [FIX] Strict P2P Scheme Enforcement (https -> wss)
    let sequencer_endpoint = env::var("SHADOW_SEQUENCER_URL").ok().map(|url| {
        if url.starts_with("https://") { url.replace("https://", "wss://") }
        else if url.starts_with("http://") { url.replace("http://", "ws://") }
        else { url }
    });

    let (mempool_listener, mempool_rx, priority_rx) = MempoolListener::new(MempoolListenerConfig {
        endpoints:          wss_urls.clone(), // Use ALL WSS keys for maximum coverage
        chain,
        min_gas_price_gwei: 0, // Base L2: capture all txs
        sequencer_endpoint: sequencer_endpoint.clone(),
        worker_count: 4,
        fetcher_count: 4,
        ..Default::default()
    }).await?;

    tokio::spawn(async move {
        if let Err(e) = mempool_listener.run().await {
            error!("❌ Mempool Listener crashed: {:?}", e);
        }
    });

    // ── Swap channel: mempool → detector ─────────────────────────────────────
    let (swap_tx, swap_rx) = mpsc::channel(4096);
    let (priority_tx, priority_rx_chan) = mpsc::channel(1024);

    tokio::spawn(async move {
        let mut m_rx = mempool_rx;
        let mut p_rx = priority_rx;
        loop {
            tokio::select! {
                Some(event) = p_rx.recv() => {
                    let _ = priority_tx.try_send(event);
                }
                Some(event) = m_rx.recv() => {
                    let _ = swap_tx.try_send(event);
                }
            }
        }
    });

    // ── Pillar C/D: Arbitrage Detector ───────────────────────────────────────
    let mut detector_config        = DetectorConfig::default();
    detector_config.chain          = chain;
    detector_config.executor_address = executor_address;
    detector_config.bribe_percent  = 50; // Pillar I: Consolidate to 50% across engine
    detector_config.scanner_threads = 16;
    detector_config.min_profit_wei = U256::from(1u64); // ⚡ 1-WEI SENSITIVITY
    detector_config.pool_limit = bootstrap_count;

    let pool_rx = pool_tx.subscribe();
    let (detector, mut opp_rx, force_tx) = ArbitrageDetector::new(
        detector_config,
        ws_provider.clone(),
        state_mirror.clone(),
        gas_feed.clone(),
        bidding_engine.clone(),
        swap_rx,
        priority_rx_chan,
        pool_rx,
    ).await;

    // Get a clone of the state before the detector is moved into the background task
    let detector_state = detector.state.clone();

    // [GHOST ACTIVATION] Pillar A+ P2P Sequencer Handler
    if let Some(ref p2p_url) = sequencer_endpoint {
        let mirror_clone = state_mirror.clone();
        let f_tx = force_tx.clone();
        let url = p2p_url.clone();
        tokio::spawn(async move {
            mirror_clone.spawn_p2p_gossip_handler(url, Some(f_tx)).await;
        });
    }

    // Block-driven mirror sync (every block) - Link to Detector
    {
        let mirror        = state_mirror.clone();
        let ws_pool_clone = ws_provider_pool.clone(); // Use the WsProviderPool
        let cb            = circuit_breaker.clone();
        let be            = bidding_engine.clone();
        let f_tx          = force_tx.clone();
        tokio::spawn(async move {
            loop {
                match ws_pool_clone.next().subscribe_blocks().await { // Use next provider from pool
                    Ok(mut stream) => {
                        info!("✅ [PILLAR A] Block stream active.");
                while let Some(block) = stream.next().await {
                    let block_num = block.number.unwrap_or_default().as_u64();
                    mirror.mark_dirty();
                    if block_num % 100 == 0 { mirror.prune_stale_pools(500); }
                    be.reset_pressure();
                    tokio::time::sleep(Duration::from_millis(200)).await; // Delay to let RPC state catch up
                    mirror.sync(Some(block.clone())).await;
                    let _ = f_tx.send(()); // Zero-latency snapshot trigger
                    cb.record_sequencer_drift(block.timestamp.as_u64());
                }
                        warn!("⚠️ [PILLAR A] Block stream ended unexpectedly. Reconnecting...");
                    }
                    Err(e) => {
                        error!("❌ [PILLAR A] Block subscription failed: {:?}. Retrying in 5s...", e);
                        // The loop will automatically try the next provider in the pool on the next iteration
                    }
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
    }

    // ── Pillar R: Auditor Maintenance ────────────────────────────────────────
    // Har 30 minute mein logs check honge aur 2000 lines se zyada hone par purana data delete hoga.
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1800));
        loop {
            interval.tick().await;
            cleanup_auditor_logs();
            audit_log("SYSTEM", "Maintenance cycle: Logs checked and cleaned if necessary.");
        }
    });

    tokio::spawn(async move { detector.run().await; });

    // ── Pillar Z: Bootstrap Existing Pools ───────────────────────────────────
    // Use the strictly loaded address from the top of main()
    // let aero_factory: Address = env::var("SHADOW_AERO_FACTORY")
    //     .expect("FATAL: SHADOW_AERO_FACTORY missing from .env. Zenith Protocol initialization failed.")
    //     .parse()
    //     .expect("FATAL: Invalid SHADOW_AERO_FACTORY format");
    
    let rpc_manager_clone = http_rpc_manager.clone();
    let pool_tx_clone = pool_tx.clone();
    let detector_state_clone = detector_state.clone();
    tokio::spawn(async move {
        // Pillar Z: Intelligent Priority Bootstrap
        let mut target_v2_factories = Vec::new();
        let mut target_aero_factory = Address::zero();

        for ((c, dex), contracts) in constants::DEX_CONTRACTS.iter() {
            if *c == chain {
                match dex {
                    DexName::Aerodrome => target_aero_factory = contracts.factory,
                    DexName::UniswapV3 | DexName::Maverick | DexName::Permit2 | DexName::CowSwap => {},
                    _ => {
                        if !target_v2_factories.contains(&contracts.factory) {
                            target_v2_factories.push(contracts.factory);
                        }
                    }
                }
            }
        }

        // Add manual overrides if provided in .env
        if let Some(manual) = manual_v2_factory {
            if !target_v2_factories.contains(&manual) { target_v2_factories.push(manual); }
        }
        if let Some(manual) = manual_aero_factory {
            target_aero_factory = manual;
        }

        // Priority 1: V3 Derivation (Handled inside detector preload)
        // We don't need to do anything here, but we ensure it has a head start.
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Priority 2: Aerodrome (High Alpha on Base)
        if !target_aero_factory.is_zero() {
            info!("🌊 [ZENITH] Bootstrapping Aerodrome (High Priority)...");
            if let Err(e) = discovery::sync_initial_pools(rpc_manager_clone.clone(), pool_tx_clone.clone(), Address::zero(), target_aero_factory, bootstrap_count / 2).await { // Distribute total bootstrap count
                error!("❌ Aerodrome Bootstrap failed: {:?}", e);
            }
        }

        // Priority 3: Other V2 Factories (Sushi, BaseSwap, etc.)
        info!("🌊 [ZENITH] Bootstrapping {} V2 factories...", target_v2_factories.len());
        let per_factory_limit = (bootstrap_count - (bootstrap_count / 2)) / target_v2_factories.len().max(1); // Remaining count divided among V2 factories
        for factory_addr in target_v2_factories {
            // [FIX] Balanced distribution to hit overall bootstrap_count target
            if let Err(e) = discovery::sync_initial_pools(rpc_manager_clone.clone(), pool_tx_clone.clone(), factory_addr, Address::zero(), per_factory_limit).await {
                error!("❌ V2 Bootstrap failed for factory {:?}: {:?}", factory_addr, e);
            }
        }

        // Pillar C: Build graph ONLY after bootstrap finishes to ensure cycles > 0
        detector_state_clone.refresh_path_cache();
    });

    info!("🚀 Sovereign Shadow LIVE — hunting alpha at nanosecond speed");

    // ── Dashboard: real gas every 10s ─────────────────────────────────────────
    {
        let gf = gas_feed.clone();
        let http_manager = http_rpc_manager.clone(); // Use the manager here
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                let (base, priority, _) = gf.current().await;
                let (b, p): (U256, U256) = if base.is_zero() {
                    let b_val = http_manager.get_next_provider().get_block(BlockNumber::Latest).await
                        .ok().flatten()
                        .and_then(|bl| bl.base_fee_per_gas)
                        .unwrap_or(U256::from(100_000u64));
                    let p_val = http_manager.get_next_provider().request::<(), U256>("eth_maxPriorityFeePerGas", ())
                        .await.unwrap_or(U256::from(100_000u64));
                    (b_val, p_val)
                } else { (base, priority) };
                let bg = b.as_u128() as f64 / 1e9;
                let pg = p.as_u128() as f64 / 1e9;
                info!("📊 [GAS] Base: {:.6} gwei | Priority: {:.6} gwei", bg, pg);
            }
        });
    }

    // ── Graceful shutdown ─────────────────────────────────────────────────────
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    // ── Pillar E: Batch Execution Engine (Optimization) ──
    let execution_semaphore = Arc::new(tokio::sync::Semaphore::new(batch_size));
    loop {
        tokio::select! {
            Some(opp) = opp_rx.recv() => {
                // [STRICT TRIGGER] Pillar V: Check both global pause AND watch-only mode
                if !constants::GLOBAL_PAUSE.load(Ordering::Relaxed) && !constants::WATCH_ONLY_MODE {
                    if executor_address == Address::zero() {
                        audit_log("PILLAR E", &format!("DRY-RUN: Opp found for {} wei, but EXECUTOR_ADDRESS is zero.", opp.expected_profit));
                        debug!("💰 [DRY-RUN] Profit: {} wei", opp.expected_profit);
                        continue;
                    }

                    // [10/10 FIX] Acquire permit BEFORE spawning task to ensure strict serial order
                    if let Ok(permit) = execution_semaphore.clone().try_acquire_owned() {
                        let exec = flash_executor.clone();
                        audit_log("PILLAR E", &format!("EXECUTION START: Attempting trade for {} wei profit. (ID: {})", opp.expected_profit, opp.id));
                        tokio::spawn(async move {
                            let _ = exec.simulate_and_execute(&opp).await;
                            drop(permit); // Permit released only after full execution or failure
                        });
                    }
                }
            }
            _ = &mut shutdown => {
                info!("🛑 Shutdown received — closing Sovereign Shadow");
                break;
            }
        }
    }

    Ok(())
}

/// Spawns parallel execution for a batch of opportunities.
fn spawn_batch_execution(batch: Vec<Opportunity>, executor: Arc<FlashLoanExecutor>, pm: Arc<ProfitManager<Provider<Ws>>>) {
    for opp in batch {
        let exec = executor.clone();
        let p_m  = pm.clone();
        tokio::spawn(async move {
            match exec.simulate_and_execute(&opp).await {
                Ok(hash) => {
                    info!("✅ [TX] Batch Confirmed: {:?}", hash);
                    if let Some(d) = opp.profit_details {
                        let _ = p_m.handle_profit(d.net_profit).await;
                    }
                }
                Err(e) => {
                    error!("❌ [FATAL] Execution Reverted: {}. Triggering localized safety pause.", e);
                }
            }
        });
    }
}
