use crate::models::{Bundle, Chain, MEVError};
use crate::constants::PRIVATE_RELAYS;
use crate::nonce_manager::NonceManager;
use crate::utils::CircuitBreaker;
use crate::state_simulator::StateSimulator;
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::SignerSync;
use alloy_primitives::{keccak256, hex, Address, U256};
use reqwest::{Client, header};
use rand::Rng;
use std::sync::Arc;
use tracing::{info, error, debug};
use serde_json::json;
use alloy::providers::RootProvider;
use alloy::transports::BoxTransport;
use crate::telemetry::TelemetryHandle;

pub struct BundleBuilderConfig {
    pub chain_id: u64,
    pub chain: Chain,
    pub signer: PrivateKeySigner,
    pub identity_signer: PrivateKeySigner,
    pub executor_address: Address,
    pub min_profit_eth: U256,
    pub relays: Vec<String>,
    pub l2_private_rpcs: Vec<String>,
    pub base_bribe_percent: u64,
    pub max_gas_price_gwei: u64,
    pub enable_simulation: bool,
    pub use_flashbots_simulation: bool,
    pub check_flash_loan: bool,
    pub relay_timeout_ms: u64,
    pub stealth_jitter: bool,
    pub use_raw_encoding: bool,
    pub nonce_recovery_blocks: u64,
    pub max_consecutive_failures: u64,
    pub pause_duration_secs: u64,
    pub ai_strategy: Option<String>,
    pub telemetry_tx: Option<Arc<TelemetryHandle>>,
}

pub struct BlockTracker {
    current_block: std::sync::atomic::AtomicU64,
}

impl BlockTracker {
    pub fn new() -> Self { Self { current_block: std::sync::atomic::AtomicU64::new(0) } }
    pub fn update(&self, block: u64) { self.current_block.store(block, std::sync::atomic::Ordering::Release); }
    pub fn current(&self) -> u64 { self.current_block.load(std::sync::atomic::Ordering::Acquire) }
}

/// Pillar E: Multi-Relay Propagation.
/// Signs and broadcasts bundles to multiple MEV relays concurrently.
pub struct BundleBuilder {
    client: Client,
    pub config: BundleBuilderConfig,
    #[allow(dead_code)]
    provider: Arc<RootProvider<BoxTransport>>,
    #[allow(dead_code)]
    nonce_manager: Arc<NonceManager>,
    #[allow(dead_code)]
    circuit_breaker: Arc<CircuitBreaker>,
    #[allow(dead_code)]
    state_simulator: Arc<StateSimulator>,
    pub block_tracker: Arc<BlockTracker>,
}

impl BundleBuilder {
    pub async fn new(
        config: BundleBuilderConfig,
        provider: Arc<RootProvider<BoxTransport>>,
        nonce_manager: Arc<NonceManager>,
        circuit_breaker: Arc<CircuitBreaker>,
        state_simulator: Arc<StateSimulator>,
    ) -> Result<Self, MEVError> {
        Ok(Self {
            client: Client::new(),
            config,
            provider,
            nonce_manager,
            circuit_breaker,
            state_simulator,
            block_tracker: Arc::new(BlockTracker::new()),
        })
    }

    pub async fn broadcast_bundle(&self, bundle: Bundle) -> Vec<Result<(), String>> {
        let txs_hex: Vec<String> = bundle.transactions.iter()
            .map(|tx| format!("0x{}", hex::encode(tx)))
            .collect();

        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "mev_sendBundle", // Target MEV-Blocker specific method
            "params": [{
                "txs": txs_hex,
                "blockNumber": format!("0x{:x}", bundle.target_block),
                "privacy": {
                    "hints": ["calldata", "logs", "default_logs", "hash"]
                }
            }]
        });

        let body = payload.to_string();
        let signature = self.sign_request(&body).await;
        
        // Pillar E: Stealth Jitter Implementation
        if self.config.stealth_jitter {
            let jitter = rand::thread_rng().gen_range(10..50);
            tokio::time::sleep(std::time::Duration::from_millis(jitter)).await;
        }

        let mut futures = Vec::new();
        for relay in PRIVATE_RELAYS.iter() {
            futures.push(self.send_to_relay(relay, body.clone(), signature.clone()));
        }

        futures::future::join_all(futures).await
    }

    async fn sign_request(&self, body: &str) -> String {
        let hash = keccak256(body.as_bytes());
        let sig = self.config.identity_signer.sign_hash_sync(&hash).expect("Signing failed");
        format!("{}:0x{}", self.config.identity_signer.address(), hex::encode(sig.as_bytes()))
    }

    async fn send_to_relay(&self, url: &str, body: String, signature: String) -> Result<(), String> {
        debug!("Sending bundle to relay: {}", url);
        
        let res = self.client.post(url)
            .header("X-Flashbots-Signature", signature)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if res.status().is_success() {
            info!("✅ Bundle accepted by relay: {}", url);
            Ok(())
        } else {
            let err_text = res.text().await.unwrap_or_default();
            error!("❌ Relay {} rejected bundle: {}", url, err_text);
            Err(err_text)
        }
    }
}