use alloy::providers::{RootProvider, Provider};
use alloy::transports::BoxTransport;
use alloy_primitives::Address;
use alloy::rpc::types::Filter;
use alloy_primitives::B256;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{error, info, warn};
use crate::factory_scanner::{NewPoolEvent, V2PoolData};
use crate::models::{Chain, DexName};
use crate::constants;

/// Pillar Z: Historical pool discovery via log scanning (Warm Start).
pub struct Discovery {
    provider: Arc<RootProvider<BoxTransport>>,
    pool_tx: broadcast::Sender<NewPoolEvent>,
    chain: Chain,
}

impl Discovery {
    pub fn new(provider: Arc<RootProvider<BoxTransport>>, pool_tx: broadcast::Sender<NewPoolEvent>, chain: Chain) -> Self {
        Self { provider, pool_tx, chain }
    }

    /// Pillar Z: Pre-seed registry with core liquid pairs to ensure immediate readiness.
    pub fn bootstrap_core_pools(&self) {
        info!("🧬 [PILLAR Z] Force-feeding Registry with CORE liquidity clusters...");
        
        if self.chain == Chain::Base {
            let core_pools = vec![
                // Primary Meme Liquidity Hubs (WETH-USDC V2/Aero)
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: constants::TOKEN_USDC, pair: constants::POOL_BASESWAP_WETH_USDC, dex_name: DexName::BaseSwap }),
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: constants::TOKEN_USDC, pair: constants::POOL_AERO_WETH_USDC, dex_name: DexName::Aerodrome }),
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: constants::TOKEN_USDC, pair: constants::POOL_SUSHI_WETH_USDC, dex_name: DexName::SushiSwap }),
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: constants::TOKEN_USDC, pair: constants::POOL_PANCAKESWAP_WETH_USDC, dex_name: DexName::PancakeSwap }),

                // Alpha Cluster: DEGEN Multi-Hop Bridges
                // In pools par competition kam hai aur triangular arb ke mauke zyada hain.
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: constants::TOKEN_AERO, pair: constants::POOL_AERO_WETH_AERO, dex_name: DexName::Aerodrome }),
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: constants::TOKEN_DEGEN, pair: constants::POOL_UNIV2_WETH_DEGEN, dex_name: DexName::UniswapV2 }),
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_USDC, token_1: constants::TOKEN_DEGEN, pair: constants::POOL_UNIV2_USDC_DEGEN, dex_name: DexName::UniswapV2 }),

                // Alpha Cluster: BRETT & Yield Tokens
                // BRETT/AERO aur BRETT/WETH ke beech price hamesha sync nahi rehta.
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: constants::TOKEN_BRETT, pair: constants::POOL_AERO_WETH_BRETT, dex_name: DexName::Aerodrome }),
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_CBETH, token_1: constants::TOKEN_WETH, pair: alloy_primitives::address!("0x2Ae3F1Ec7F1F5012CFEab0185bfc7aa3cf0DEc22"), dex_name: DexName::BaseSwap }),

                // The "Scavenger Bridge": Degen to Ecosystem
                // Ye rasta bade bots kabhi scan nahi karte.
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_DEGEN, token_1: constants::TOKEN_AERO, pair: alloy_primitives::address!("0x532f27101965dd16442E59d40670Fa5ad5f3fe91"), dex_name: DexName::BaseSwap }),
                
                // Gaming/Meme Alpha Bridge
                NewPoolEvent::V2(V2PoolData { 
                    token_0: constants::TOKEN_WETH, 
                    token_1: alloy_primitives::address!("0x4ed4E862860beD51a9570b96d89aF5E1B0Efefed"), // DEGEN 
                    pair: alloy_primitives::address!("0x3D2d7681335A74Be482D207137f814bA688849E8"), 
                    dex_name: DexName::UniswapV2 
                }),
            ];

            let mut count = 0;
            for pool_event in core_pools {
                if self.pool_tx.send(pool_event).is_ok() {
                    count += 1;
                }
            }
            info!("✅ [PILLAR Z] Force-fed {} core clusters to Base registry. Graph is now primed.", count);
        }
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

            let v2_topic = B256::from(constants::EVENT_V2_PAIR_CREATED);

            let mut current_start = start_block;
            let step = 500; // Smaller chunks to prevent timeouts and RPC rejections

            while current_start < current_block {
                let current_end = (current_start + step).min(current_block);
                let filter = Filter::new()
                    .from_block(current_start)
                    .to_block(current_end)
                    .event_signature(vec![v2_topic]); // Target only V2-style Meme Factories

                match provider.get_logs(&filter).await {
                    Ok(logs) => {
                        let mut count = 0;
                        for log in logs {
                            let data = log.data().data.as_ref();
                            let topics = log.topics();
                            if !topics.is_empty() && topics[0] == v2_topic && topics.len() >= 3 {
                                let token0 = Address::from_word(topics[1]);
                                let token1 = Address::from_word(topics[2]);
                                if data.len() >= 32 {
                                    let pair = Address::from_slice(&data[12..32]);
                                    let _ = pool_tx.send(NewPoolEvent::V2(V2PoolData { 
                                        token_0: token0, token_1: token1, pair, dex_name: DexName::UniswapV2 
                                    }));
                                    count += 1;
                                }
                            }
                        }
                        if count > 0 { info!("✅ [PILLAR Z] Injected {} pools from historical blocks.", count); }
                    }
                    Err(e) => {
                        let err_msg = e.to_string();
                        if err_msg.contains("-32002") || err_msg.contains("Archive") || err_msg.contains("limit") {
                            warn!("⚠️ [PILLAR Z] Historical scan restricted by RPC plan (Chainstack/Alchemy). Discovery will rely on bootstrap pools and real-time logs.");
                            break;
                        }
                        error!("❌ [PILLAR Z] Warm Start chunk scan failed: {}", err_msg);
                    }
                }
                current_start = current_end + 1;
            }
            info!("🏁 [PILLAR Z] Warm Start process complete.");
        });
    }
}
