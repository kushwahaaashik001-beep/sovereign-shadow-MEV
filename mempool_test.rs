use the_sovereign_shadow::mempool_listener::{MempoolListener, MempoolListenerConfig};
use dotenv::dotenv;
use std::env;
use ethers::types::Chain;
use tracing::{info, error, Level};
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();

    // Setup high-performance logging for the mempool audit
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    info!("🚀 Initializing 'The Sovereign Shadow' Mempool Audit...");

    let wss_url = env::var("SHADOW_WS_URL").expect("SHADOW_WS_URL required in .env");
    let chain_name = env::var("CHAIN").unwrap_or_else(|_| "base".to_string());
    let chain = match chain_name.as_str() {
        "base"     => Chain::Base,
        "arbitrum" => Chain::Arbitrum,
        _          => Chain::Mainnet,
    };

    let config = MempoolListenerConfig {
        endpoints: vec![wss_url],
        chain,
        ..Default::default()
    };

    let (mempool_listener, mut mempool_rx, mut priority_rx) = MempoolListener::new(config).await?;

    info!("🛰️  Mempool Listener Online. Monitoring live swaps on {:?}...", chain);

    tokio::spawn(async move {
        if let Err(e) = mempool_listener.run().await {
            error!("❌ Mempool Listener crashed: {:?}", e);
        }
    });

    loop {
        tokio::select! {
            Some(event) = priority_rx.recv() => {
                info!("🐋 [PRIORITY] Whale Swap: {:?} -> {:?} | Router: {:?}", 
                    event.swap_info.token_in, event.swap_info.token_out, event.swap_info.router);
            }
            Some(event) = mempool_rx.recv() => {
                info!("📦 [MEMPOOL] Swap Detected: {:?} -> {:?} | Router: {:?}", 
                    event.swap_info.token_in, event.swap_info.token_out, event.swap_info.router);
            }
        }
    }
}