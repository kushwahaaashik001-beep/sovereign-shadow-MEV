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
                // AI & Social Alpha Hubs
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: alloy_primitives::address!("0x0bFbCF9fa4f9C56B0F40a671Ad40E0805A091865"), pair: alloy_primitives::address!("0x3062ad446da2cfdb10266e06bee30f33ba2a6b41"), dex_name: DexName::Aerodrome }), // VIRTUAL/WETH
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: alloy_primitives::address!("0x4F9Fd6Be4a90f2620860d680c0d4d5Fb53d1A84E"), pair: alloy_primitives::address!("0x15263a6a1251d75cdf2de83a1251d75cdf2de83a"), dex_name: DexName::UniswapV2 }), // AIXBT
                
                // Ecosystem Triangular Seeds
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_DEGEN, token_1: constants::TOKEN_AERO, pair: alloy_primitives::address!("0x532f27101965dd16442E59d40670Fa5ad5f3fe91"), dex_name: DexName::BaseSwap }),
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: constants::TOKEN_LUNA, pair: alloy_primitives::address!("0x4200000000000000000000000000000000000006"), dex_name: DexName::Aerodrome }),
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: constants::TOKEN_AI16Z, pair: alloy_primitives::address!("0x30c90069678174577B0Ac49969D7070F7915B597"), dex_name: DexName::UniswapV2 }),
                
                // Multi-DEX Path Inflation (AlienBase, SwapBased, Sushi)
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: constants::TOKEN_USDC, pair: alloy_primitives::address!("0x04C9F118A4864700721A163744021d21DB27c11f"), dex_name: DexName::UniswapV2 }), // SwapBased
                NewPoolEvent::V2(V2PoolData { token_0: constants::TOKEN_WETH, token_1: constants::TOKEN_USDC, pair: alloy_primitives::address!("0x3D2d7681335A74Be482D207137f814bA688849E8"), dex_name: DexName::UniswapV2 }), // AlienBase
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
        let pool = self.http_pool.clone();
        
        // Background task to prevent blocking the main engine startup
        tokio::spawn(async move {
            let (_, provider) = pool.get_head(0);
            let current_block = provider.get_block_number().await.unwrap_or_default();
            // [DEEP-DISCOVERY] Scan last 5,000 blocks in batches of 500 to respect Alchemy limits.
            let lookback = 5000;
            let mut start_block = current_block.saturating_sub(lookback);

            let v2_topic = B256::from(constants::EVENT_V2_PAIR_CREATED);
            let v3_topic = B256::from(constants::EVENT_V3_POOL_CREATED);
            let aero_topic = alloy_primitives::fixed_bytes!("0x212847ad1f2f1ad0d76077f4a7f5f3e728cc2ac818eb64fed8004e115fbcca67");

            let mut total_discovered = 0;
            while start_block < current_block {
                let (_, provider) = pool.next(); // Rotate key for every batch
                let end_batch = (start_block + 500).min(current_block);
                let filter = Filter::new()
                    .from_block(start_block)
                    .to_block(end_batch)
                    .event_signature(vec![v2_topic, v3_topic, aero_topic]);

                if let Ok(logs) = provider.get_logs(&filter).await {
                    for log in logs {
                        if let Some(event) = Self::parse_historical_log(&log, v2_topic, v3_topic, aero_topic) {
                            let _ = pool_tx.send(event);
                            total_discovered += 1;
                        }
                    }
                }
                start_block = end_batch + 1;
                // Anti-RateLimit: 200ms delay between batches
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
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
