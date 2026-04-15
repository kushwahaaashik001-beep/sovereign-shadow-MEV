use alloy::providers::{RootProvider, Provider};
use alloy::transports::BoxTransport;
use alloy_primitives::Address;
use alloy::rpc::types::Filter;
use alloy_primitives::fixed_bytes;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, warn, error};
use crate::factory_scanner::{NewPoolEvent, V2PoolData, V3PoolData};
use crate::models::{Chain, DexName};

/// Pillar Z: Historical pool discovery via log scanning (Warm Start).
pub struct Discovery {
    provider: Arc<RootProvider<BoxTransport>>,
    pool_tx: broadcast::Sender<NewPoolEvent>,
    _chain: Chain,
}

impl Discovery {
    pub fn new(provider: Arc<RootProvider<BoxTransport>>, pool_tx: broadcast::Sender<NewPoolEvent>, chain: Chain) -> Self {
        Self { provider, pool_tx, _chain: chain }
    }

    pub async fn warm_start(&self) {
        info!("🕯️ [PILLAR Z] Warm Start: Scanning historical logs for existing pools...");
        
        let pool_tx = self.pool_tx.clone();
        let provider = self.provider.clone();
        
        // Background task to prevent blocking the main engine startup
        tokio::spawn(async move {
            let current_block = provider.get_block_number().await.unwrap_or_default();
            // Further reduced range to 500 blocks for free-tier RPC stability
            let start_block = current_block.saturating_sub(500); // Keep this small to avoid RPC limits

            let v2_topic = fixed_bytes!("0d3648bd0f6ba80134a33ba9275ac585d9d315f0ad8355cd33e8bb5511a35a1d");
            let v3_topic = fixed_bytes!("783cca1c0412dd0d695e784d03c4399881a4e8a1f8e136325997d9cf1673e728");

            let mut current_start = start_block;
            let step = 500; // Smaller chunks to prevent timeouts and RPC rejections

            while current_start < current_block {
                let current_end = (current_start + step).min(current_block);
                let filter = Filter::new()
                    .from_block(current_start)
                    .to_block(current_end)
                    .event_signature(vec![v2_topic, v3_topic]);

                match provider.get_logs(&filter).await {
                    Ok(logs) => {
                        let mut count = 0;
                        for log in logs {
                            let data = log.data().data.as_ref();
                            if !log.topics().is_empty() && log.topics()[0] == v2_topic && log.topics().len() >= 3 {
                                let token0 = Address::from_word(log.topics()[1]);
                                let token1 = Address::from_word(log.topics()[2]);
                                if data.len() >= 32 {
                                    let pair = Address::from_slice(&data[12..32]);
                                    let _ = pool_tx.send(NewPoolEvent::V2(V2PoolData { 
                                        token_0: token0, token_1: token1, pair, dex_name: DexName::UniswapV2 
                                    }));
                                    count += 1;
                                }
                            } else if !log.topics().is_empty() && log.topics()[0] == v3_topic && log.topics().len() >= 4 {
                                let token0 = Address::from_word(log.topics()[1]);
                                let token1 = Address::from_word(log.topics()[2]);
                                if data.len() >= 64 {
                                    // Fee is indexed in topics[3], not in data
                                    let fee = u32::from_be_bytes([0, log.topics()[3][29], log.topics()[3][30], log.topics()[3][31]]);
                                    let pool = Address::from_slice(&data[44..64]);
                                    let _ = pool_tx.send(NewPoolEvent::V3(V3PoolData { 
                                        token_0: token0, token_1: token1, fee, pool, dex_name: DexName::UniswapV3 
                                    }));
                                    count += 1;
                                }
                            }
                        }
                        if count > 0 { info!("✅ [PILLAR Z] Injected {} pools from historical blocks.", count); }
                    }
                    Err(e) => {
                        error!("❌ [PILLAR Z] Warm Start chunk scan failed: {}", e);
                        if e.to_string().contains("-32002") || e.to_string().contains("Archive") || e.to_string().contains("limit") {
                            warn!("⚠️ [PILLAR Z] Historical scan restricted. Continuing with real-time discovery only.");
                            break;
                        }
                    }
                }
                current_start = current_end + 1;
            }
            info!("🏁 [PILLAR Z] Warm Start process complete.");
        });
    }
}
