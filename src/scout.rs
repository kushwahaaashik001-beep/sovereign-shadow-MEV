//! Space A: The Global Scout
//! Lightweight Rust client to monitor mempool and stream raw transactions to the Sniper Brain via gRPC.

use alloy::providers::{Provider, ProviderBuilder, WsConnect};
use futures_util::StreamExt;
use std::env;
use std::error::Error;
use std::time::{SystemTime, UNIX_EPOCH, Duration};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{transport::Channel, metadata::MetadataValue, Request};
use tracing::{error, info};

pub mod hydra {
    tonic::include_proto!("hydra");
}

use hydra::hydra_network_client::HydraNetworkClient;
use hydra::RawOpportunity;

fn main() -> Result<(), Box<dyn Error>> {
    let core_ids = core_affinity::get_core_ids().expect("Failed to get core IDs");
    let core_counter = Arc::new(AtomicUsize::new(0));

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(core_ids.len())
        .enable_all()
        .on_thread_start(move || {
            let idx = core_counter.fetch_add(1, Ordering::SeqCst) % core_ids.len();
            core_affinity::set_for_current(core_ids[idx]);
        })
        .build()?;

    runtime.block_on(run_scout())
}

async fn run_scout() -> Result<(), Box<dyn Error>> {
    dotenv::dotenv().ok();

    // Initialize structured logging for the Scout
    tracing_subscriber::fmt::init();

    let scout_id = env::var("SCOUT_ID").unwrap_or_else(|_| "global-scout-01".to_string());
    let sniper_url = env::var("SNIPER_URL").unwrap_or_else(|_| "http://127.0.0.1:50051".to_string());
    // Space A Optimization: Only listen, never sign.
    let ws_url = env::var("SHADOW_WS_URL")
        .or_else(|_| env::var("SHADOW_WS_URL_1"))
        .expect("👁️ Scout needs SHADOW_WS_URL");
    
    // Fast Auth: Resolve unused imports by implementing security
    let auth_token = env::var("SHADOW_AUTH_TOKEN").or_else(|_| env::var("ACCESS_TOKEN")).expect("🚀 Scout needs Auth Token");
    let token_metadata = MetadataValue::from_str(&auth_token)?;

    info!("👁️ [SCOUT {}] Initializing Global Eye", scout_id);

    // 1. Nervous System: Connect to the Central Sniper Brain (Space B)
    // connect_lazy ensures the Scout stays alive even if the Sniper is temporarily down
    // Optimization: Add keep-alive and TCP tuning
    let channel = Channel::from_shared(sniper_url)?
        .tcp_nodelay(true)
        .tcp_keepalive(Some(Duration::from_secs(60)))
        .http2_keep_alive_interval(Duration::from_secs(60))
        .connect_lazy();

    let mut client = HydraNetworkClient::with_interceptor(channel, move |mut req: Request<()>| {
        req.metadata_mut().insert("x-shadow-token", token_metadata.clone());
        Ok(req)
    });

    // 2. Communication: Setup gRPC stream for high-speed binary propagation
    let (grpc_tx, grpc_rx) = mpsc::channel(4096);
    let request_stream = ReceiverStream::new(grpc_rx);

    // Spawn a task to handle the persistent gRPC stream
    tokio::spawn(async move {
        info!("📡 [SCOUT] Opening persistent gRPC stream to Sniper...");
        if let Err(e) = client.stream_opportunities(request_stream).await {
            error!("❌ [SCOUT] gRPC connection error: {}", e);
        }
    });

    // 3. Surveillance: Listen to P2P/WS mempool transactions
    info!("🔌 Connecting to mempool feed: {}", &ws_url);
    let ws = WsConnect::new(ws_url);
    let provider = ProviderBuilder::new().on_ws(ws).await?;

    // Subscribe to full pending transactions for maximum data granularity
    let sub = provider.subscribe_full_pending_transactions().await?;
    let mut stream = sub.into_stream();

    info!("🚀 [SCOUT {}] Surveillance Active - Streaming data to Sniper", scout_id);

    while let Some(tx) = stream.next().await {
        // Space A Filter: We only gossip about contract interactions (calls)
        if let Some(to) = tx.to {
            if !tx.input.is_empty() {
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;

                // HIGH-SPEED OPTIMIZATION: Take ownership of the input Vec instead of cloning
                // alloy_primitives::Bytes(Vec<u8>) -> move Vec<u8>
                let input_vec = tx.input.0; 

                let opportunity = RawOpportunity {
                    tx_hash: bytes::Bytes::copy_from_slice(tx.hash.as_slice()), // Efficient Copy
                    pool_address: bytes::Bytes::copy_from_slice(to.as_slice()), // Efficient Copy
                    data_payload: input_vec.into(), // Convert inner Vec to Bytes
                    timestamp: timestamp as i64,
                    gas_price: tx.gas_price.unwrap_or_default() as u64,
                };

                // Binary push to the Sniper. Non-blocking to maintain surveillance speed.
                let _ = grpc_tx.try_send(opportunity);
            }
        }
    }

    Ok(())
}