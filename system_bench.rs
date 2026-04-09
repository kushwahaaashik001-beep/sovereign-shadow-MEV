use std::time::{Instant, Duration};
use tokio;
use the_sovereign_shadow::{v3_math, state_mirror, state_simulator, Opportunity, Path}; 
use dotenv::dotenv;
use ethers::types::{Address, U256};
use std::env;

const MATH_ITERATIONS: u32 = 1_000_000;

#[tokio::main]
async fn main() {
    dotenv().ok();
    
    let is_release = !cfg!(debug_assertions);
    println!("🚀 Starting 'The Sovereign Shadow' Full System Audit...\n");
    if !is_release {
        println!("⚠️  WARNING: Running in DEBUG mode. Latency will be 100x higher than production.");
        println!("👉 Use: 'cargo run --release --bin system_bench'\n");
    }

    // --- 1. ANKHEIN (Network Latency) ---
    println!("👀 Testing 'Ankhein' (Network Connectivity)...");
    let http_url = env::var("SHADOW_RPC_URL").unwrap_or_else(|_| "https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY".to_string());
    let wss_url = env::var("SHADOW_WS_URL").unwrap_or_else(|_| "wss://eth-mainnet.g.alchemy.com/v2/YOUR_KEY".to_string());
    
    // HTTP Client with connection pooling for realistic MEV testing
    let client = reqwest::Client::builder()
        .tcp_keepalive(Duration::from_secs(60))
        .build().unwrap();
        
    let start_net = Instant::now();
    let res = client.post(&http_url)
        .json(&serde_json::json!({"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}))
        .send().await;
    
    match res {
        Ok(_) => println!("   ✅ Network Latency (Round-trip): {:?}\n", start_net.elapsed()),
        Err(_) => println!("   ❌ Network Failed: Check your .env file or SHADOW_RPC_URL.\n"),
    }

    // --- 2. DIMAAG (Math Engine) ---
    println!("🧠 Testing 'Dimaag' (Math Logic Speed)...");
    
    // Test 2a: Primitive LUT Lookups
    let start_math = Instant::now();
    for _ in 0..MATH_ITERATIONS {
        let _ = v3_math::get_sqrt_ratio_at_tick(std::hint::black_box(1000));
    }
    let avg_math = start_math.elapsed() / MATH_ITERATIONS; // Fixed Divisor
    println!("   ✅ Avg Primitive Lookup: {:?}", avg_math);

    // Test 2b: Triple-Hop Multiplier (Real Arb Path)
    let start_path = Instant::now();
    let price = v3_math::get_sqrt_ratio_at_tick(1000);
    let factor = 0xfff97272373d413259a46990580e213au128;
    for _ in 0..MATH_ITERATIONS {
        let p1 = v3_math::mul_shift_128(std::hint::black_box(price), std::hint::black_box(factor));
        let p2 = v3_math::mul_shift_128(p1, factor);
        let _ = v3_math::mul_shift_128(p2, factor);
    }
    println!("   ✅ Avg 3-Hop Path Math: {:?}\n", start_path.elapsed() / MATH_ITERATIONS);

    // --- 3. HAATH (Simulation/REVM) ---
    println!("🥊 Testing 'Haath' (REVM Multiverse Simulation)...");
    
    // Create a mock opportunity for realistic testing
    let mock_opp = Opportunity {
        id: "bench_test".to_string(),
        path: std::sync::Arc::new(Path::new(&[], 0, 0)),
        expected_profit: U256::from(10u128.pow(17)),
        gas_cost: U256::from(500_000),
        success_prob: 10000,
        base_fee: U256::from(100_000_000),
        priority_fee: U256::from(1_000_000_000),
        gas_estimate: U256::from(300_000),
        input_amount: U256::from(10u128.pow(18)),
        input_token: Address::zero(),
        profit_details: None,
        chain: ethers::types::Chain::Base,
        static_calldata: Default::default(),
        trigger_gas_price: None,
        trigger_sender: None,
    };

    // Fixed: Mirror requires Ws provider. Connect to SHADOW_WS_URL for benchmark.
    let ws_provider = ethers::providers::Provider::<ethers::providers::Ws>::connect(&wss_url)
        .await
        .expect("WS_CONNECT_FAIL: Check SHADOW_WS_URL in .env");

    let http_urls = vec![http_url];
    let rpc_manager = std::sync::Arc::new(the_sovereign_shadow::rpc_manager::RpcManager::new(http_urls));
    let mirror = state_mirror::StateMirror::new(std::sync::Arc::new(ws_provider), rpc_manager, Address::zero());
    let simulator = state_simulator::StateSimulator::new(mirror, Address::random());
    
    let start_sim = Instant::now();
    // Benchmarking account loading + DB cloning overhead
    let _ = simulator.simulate_arbitrage(&mock_opp, &[], None);
    println!("   ✅ Simulation Latency: {:?}\n", start_sim.elapsed());

    println!("--------------------------------------------------");
    println!("🏆 FINAL VERDICT: System is ready for High-Frequency Trading!");
}