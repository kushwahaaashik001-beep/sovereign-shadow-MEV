use alloy::providers::Provider;
use alloy_primitives::Address;
use alloy::rpc::types::Filter;
use alloy_primitives::B256;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::info;
use crate::factory_scanner::{NewPoolEvent, V2PoolData};
use crate::models::{Chain, DexName};
use crate::constants;

/// Pillar Z: Historical pool discovery via log scanning (Warm Start).
pub struct Discovery {
    http_pool: Arc<crate::WsProviderPool>, // Use pool instead of single provider
    pool_tx: broadcast::Sender<NewPoolEvent>,
    chain: Chain,
}

impl Discovery {
    pub fn new(http_pool: Arc<crate::WsProviderPool>, pool_tx: broadcast::Sender<NewPoolEvent>, chain: Chain) -> Self {
        Self { http_pool, pool_tx, chain }
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

                // Core Alpha: DEGEN & AERO (Top 10 Essential Pairs)
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: constants::TOKEN_AERO, pair: constants::POOL_AERO_WETH_AERO, dex_name: DexName::Aerodrome }),
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: constants::TOKEN_DEGEN, pair: constants::POOL_UNIV2_WETH_DEGEN, dex_name: DexName::UniswapV2 }),
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_USDC, token_1: constants::TOKEN_DEGEN, pair: constants::POOL_UNIV2_USDC_DEGEN, dex_name: DexName::UniswapV2 }),
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: constants::TOKEN_BRETT, pair: constants::POOL_AERO_WETH_BRETT, dex_name: DexName::Aerodrome }),
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: constants::TOKEN_CBETH, pair: alloy_primitives::address!("0x2Ae3F1Ec7F1F5012CFEab0185bfc7aa3cf0DEc22"), dex_name: DexName::BaseSwap }),
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: constants::TOKEN_VIRTUAL, pair: alloy_primitives::address!("0x0bFbCF9fa4f9C56B0F40a671Ad40E0805A091865"), dex_name: DexName::Aerodrome }),
                
                // Blue-Chip Expansions (LINK, cbBTC, UNI)
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: constants::TOKEN_CBBTC, pair: alloy_primitives::address!("0x4C36388bE6F416A29C8d8Eee819bb35ed3737a01"), dex_name: DexName::UniswapV3 }),
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: constants::TOKEN_LINK, pair: alloy_primitives::address!("0xf891170fd2a3634f0E215578566473D580fC2f9d"), dex_name: DexName::SushiSwap }),
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: constants::TOKEN_UNI, pair: alloy_primitives::address!("0x38924e59761668b37510074929346d5f1370cc9c"), dex_name: DexName::UniswapV3 }),
                
                // Multi-DEX Path Inflation (AlienBase, SwapBased, Sushi)
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: constants::TOKEN_DAI, pair: alloy_primitives::address!("0x04C9F118A4864700721A163744021d21DB27c11f"), dex_name: DexName::UniswapV2 }),
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: constants::TOKEN_USDT, pair: alloy_primitives::address!("0x3D2d7681335A74Be482D207137f814bA688849E8"), dex_name: DexName::UniswapV2 }),
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
        let http_pool_for_discovery = self.http_pool.clone();
        
        // Background task to prevent blocking the main engine startup
        tokio::spawn(async move {
            // [SMART-FILTER] Providers usually block anonymous log filtering.
            // Adding explicit factory addresses to the filter makes it 100x lighter.
            let factory_addresses = vec![
                constants::BASE_AERODROME_FACTORY,
                constants::BASE_BASESWAP_FACTORY,
                constants::BASE_PANCAKESWAP_FACTORY,
                constants::BASE_SUSHISWAP_FACTORY,
                constants::BASE_MAVERICK_FACTORY,
                alloy_primitives::address!("0x33128a8fC170d56ED8068699e168a9A301C035De"), // UniV3
                alloy_primitives::address!("0x04C9F118A4864700721A163744021d21DB27c11f"), // SwapBased
                alloy_primitives::address!("0x3D2d7681335A74Be482D207137f814bA688849E8"), // AlienBase
            ];

            let (_, provider) = http_pool_for_discovery.get_head(0);
            // Explicitly define type to help compiler inference
            let current_block = provider.get_block_number().await.unwrap_or_default();
            // [LIGHT-DISCOVERY] Scanning only recent blocks to stay within free tier log limits
            let lookback = 2000; 
            let mut start_block = current_block.saturating_sub(lookback);

            let v2_topic = B256::from(constants::EVENT_V2_PAIR_CREATED);
            let v3_topic = B256::from(constants::EVENT_V3_POOL_CREATED);
            let aero_topic = alloy_primitives::fixed_bytes!("0x212847ad1f2f1ad0d76077f4a7f5f3e728cc2ac818eb64fed8004e115fbcca67");

            let mut total_discovered = 0;
            while start_block < current_block {
                let (idx, provider) = http_pool_for_discovery.next(); // Rotate key for every batch
                let end_batch = (start_block + 250).min(current_block); // Tiny batches for stability
                let filter = Filter::new()
                    .address(factory_addresses.clone())
                    .from_block(start_block)
                    .to_block(end_batch)
                    .event_signature(vec![v2_topic, v3_topic, aero_topic]);

                match provider.get_logs(&filter).await {
                    Ok(logs) => {
                        for log in logs {
                            if let Some(event) = Self::parse_historical_log(&log, v2_topic, v3_topic, aero_topic) {
                                let _ = pool_tx.send(event);
                                total_discovered += 1;
                            }
                        }
                        start_block = end_batch + 1;
                    }
                    Err(_) => {
                        http_pool_for_discovery.mark_unhealthy(idx, 10);
                        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                    }
                }
                // Heavy jitter: 5 seconds between log requests
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
            info!("🏁 [PILLAR Z] Warm Start complete. Discovered {} historical pools.", total_discovered);
        });
    }

    fn parse_historical_log(log: &alloy::rpc::types::Log, v2: B256, v3: B256, aero: B256) -> Option<NewPoolEvent> {
        let topics = log.topics();
        if topics.is_empty() { return None; }
        let data = log.data().data.as_ref();

        if topics[0] == v2 && topics.len() >= 3 && data.len() >= 32 {
            Some(NewPoolEvent::V2(V2PoolData { 
                token_0: Address::from_word(topics[1]), token_1: Address::from_word(topics[2]), 
                pair: Address::from_slice(&data[12..32]), dex_name: DexName::UniswapV2 
            }))
        } else if topics[0] == v3 && topics.len() >= 3 && data.len() >= 64 {
            Some(NewPoolEvent::V3(crate::factory_scanner::V3PoolData {
                pool: Address::from_slice(&data[44..64]), token_0: Address::from_word(topics[1]), 
                token_1: Address::from_word(topics[2]), fee: 0, dex_name: DexName::UniswapV3
            }))
        } else if topics[0] == aero && topics.len() >= 3 && data.len() >= 32 {
            Some(NewPoolEvent::V2(V2PoolData { 
                token_0: Address::from_word(topics[1]), token_1: Address::from_word(topics[2]), 
                pair: Address::from_slice(&data[12..32]), dex_name: DexName::Aerodrome 
            }))
        } else { None }
    }
}
