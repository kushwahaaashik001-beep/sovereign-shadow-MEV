use alloy::providers::{RootProvider, Provider};
use alloy::transports::BoxTransport;
use alloy_primitives::{Address, B256};
use alloy::rpc::types::Filter;
use alloy_primitives::fixed_bytes;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, error};
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
            let start_block = current_block.saturating_sub(50000); 

            let v2_topic = fixed_bytes!("0d3648bd0f6ba80134a33ba9275ac585d9d315f0ad8355cd33e8bb5511a35a1d");
            let v3_topic = fixed_bytes!("783cca1c0412dd0d695e784d03c4399881a4e8a1f8e136325997d9cf1673e728");

            let filter = Filter::new()
                .from_block(start_block)
                .to_block(current_block)
                .event_signature(vec![v2_topic, v3_topic]);

            match provider.get_logs(&filter).await {
                Ok(logs) => {
                    let mut count = 0;
                    for log in logs {
                        if !log.topics().is_empty() && log.topics()[0] == v2_topic && log.topics().len() >= 3 {
                            let token0 = Address::from_word(log.topics()[1]);
                            let token1 = Address::from_word(log.topics()[2]);
                            let pair = Address::from_word(B256::from_slice(&log.data().data[12..32]));
                            let _ = pool_tx.send(NewPoolEvent::V2(V2PoolData { 
                                token_0: token0, token_1: token1, pair, dex_name: DexName::UniswapV2 
                            }));
                            count += 1;
                        } else if !log.topics().is_empty() && log.topics()[0] == v3_topic && log.topics().len() >= 4 {
                            let token0 = Address::from_word(log.topics()[1]);
                            let token1 = Address::from_word(log.topics()[2]);
                            let fee = u32::from_be_bytes([0, log.data().data[29], log.data().data[30], log.data().data[31]]);
                            let pool = Address::from_word(B256::from_slice(&log.data().data[44..64]));
                            let _ = pool_tx.send(NewPoolEvent::V3(V3PoolData { 
                                token_0: token0, token_1: token1, fee, pool, dex_name: DexName::UniswapV3 
                            }));
                            count += 1;
                        }
                    }
                    info!("✅ [PILLAR Z] Warm Start complete. Injected {} pools into graph.", count);
                }
                Err(e) => error!("❌ [PILLAR Z] Warm Start log scan failed: {}", e),
            }
        });
    }
}
