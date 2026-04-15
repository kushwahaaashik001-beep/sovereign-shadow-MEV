#![allow(dead_code)]
use crate::models::{Chain, DexName};
use crate::constants;
use alloy_primitives::{Address, B256};
use alloy::rpc::types::Filter;
use alloy::providers::Provider;
use futures_util::stream::StreamExt;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, warn, error};

// Aerodrome Factory PoolCreated Topic
pub const EVENT_AERO_POOL_CREATED: [u8; 32] = alloy_primitives::fixed_bytes!("0x212847ad1f2f1ad0d76077f4a7f5f3e728cc2ac818eb64fed8004e115fbcca67").0;

#[derive(Debug, Clone, Copy)]
pub struct V2PoolData {
    pub pair: Address,
    pub token_0: Address,
    pub token_1: Address,
    pub dex_name: DexName,
}

#[derive(Debug, Clone, Copy)]
pub struct V3PoolData {
    pub pool: Address,
    pub token_0: Address,
    pub token_1: Address,
    pub fee: u32,
    pub dex_name: DexName,
}

#[derive(Debug, Clone, Copy)]
pub enum NewPoolEvent {
    V2(V2PoolData),
    V3(V3PoolData),
}

pub struct FactoryScanner {
    ws_provider_pool: Arc<crate::WsProviderPool>,
    pool_tx: broadcast::Sender<NewPoolEvent>,
    chain: Chain,
}

impl FactoryScanner {
    pub fn new(
        ws_provider_pool: Arc<crate::WsProviderPool>,
        pool_tx: broadcast::Sender<NewPoolEvent>,
        chain: Chain,
    ) -> Self {
        Self { ws_provider_pool, pool_tx, chain }
    }

    pub async fn run(&self) {
        info!("🚀 [Pillar Z: Factory Scanner] Initializing for chain {:?}", self.chain);

        let mut factory_map = std::collections::HashMap::new();
        self.setup_factories(&mut factory_map);

        let factory_addresses: Vec<Address> = factory_map.keys().cloned().collect();
        if factory_addresses.is_empty() {
            warn!("⚠️ No factory addresses configured for scanner on {:?}", self.chain);
            return;
        }

        let filter = Filter::default()
            .address(factory_addresses)
            .event_signature(vec![
                B256::from(constants::EVENT_V2_PAIR_CREATED),
                B256::from(constants::EVENT_V3_POOL_CREATED),
                B256::from(EVENT_AERO_POOL_CREATED),
            ]);

        loop {
            let provider = self.ws_provider_pool.next();
            debug_assert!(provider.get_chain_id().await.is_ok());

            match provider.subscribe_logs(&filter.clone()).await {
                Ok(sub) => {
                    info!("📡 [Factory Scanner] Subscribed to new pool logs");
                    let mut stream = sub.into_stream();
                    while let Some(log) = stream.next().await {
                        if let Some(event) = self.parse_factory_log(&log, &factory_map) {
                            info!("✨ New Pool Detected: {:?}", event);
                            let _ = self.pool_tx.send(event);
                        }
                    }
                }
                Err(e) => {
                    error!("❌ Factory Scanner subscription failed: {}. Retrying in 5s...", e);
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        }
    }

    fn setup_factories(&self, map: &mut std::collections::HashMap<Address, DexName>) {
        match self.chain {
            Chain::Base => {
                map.insert(constants::BASE_AERODROME_FACTORY, DexName::Aerodrome);
                map.insert(constants::BASE_BASESWAP_FACTORY, DexName::BaseSwap);
                map.insert(constants::BASE_PANCAKESWAP_FACTORY, DexName::PancakeSwap);
                map.insert(constants::BASE_SUSHISWAP_FACTORY, DexName::SushiSwap);
                map.insert(constants::BASE_MAVERICK_FACTORY, DexName::Maverick);
                // Uniswap V3 Factory on Base
                map.insert("0x33128a8fC170d56ED8068699e168a9A301C035De".parse().unwrap(), DexName::UniswapV3);
            }
            Chain::Mainnet => {
                map.insert(constants::UNISWAP_V2_FACTORY, DexName::UniswapV2);
                map.insert("0x1F98431c8aD98523631AE4a59f267346ea31F984".parse().unwrap(), DexName::UniswapV3);
            }
            _ => warn!("Factory addresses not fully mapped for chain {:?}", self.chain),
        }
    }

    fn parse_factory_log(
        &self, 
        log: &alloy::rpc::types::Log, 
        factory_map: &std::collections::HashMap<Address, DexName>
    ) -> Option<NewPoolEvent> {
        let topics = log.topics();
        if topics.is_empty() { return None; }

        let dex_name = factory_map.get(&log.address())?;
        let topic0 = topics[0];

        if topic0 == constants::EVENT_V2_PAIR_CREATED {
            // V2: PairCreated(address indexed token0, address indexed token1, address pair, uint)
            if topics.len() < 3 { return None; }
            let token_0 = Address::from_word(topics[1]);
            let token_1 = Address::from_word(topics[2]);
            let data = log.data().data.as_ref();
            let pair = Address::from_slice(data.get(12..32)?);

            return Some(NewPoolEvent::V2(V2PoolData {
                pair, token_0, token_1, dex_name: *dex_name,
            }));
        }

        if topic0 == EVENT_AERO_POOL_CREATED && dex_name == &DexName::Aerodrome {
            // Aerodrome: PoolCreated(address indexed token0, address indexed token1, bool indexed stable, address pool, uint256)
            if topics.len() < 4 { return None; }
            let token_0 = Address::from_word(topics[1]);
            let token_1 = Address::from_word(topics[2]);
            let _is_stable = topics[3] != B256::ZERO;
            let data = log.data().data.as_ref();
            let pool = Address::from_slice(data.get(12..32)?);

            return Some(NewPoolEvent::V2(V2PoolData {
                pair: pool, token_0, token_1, dex_name: DexName::Aerodrome,
            }));
        } 
        
        if topic0 == constants::EVENT_V3_POOL_CREATED {
            // V3: PoolCreated(address indexed token0, address indexed token1, uint24 indexed fee, int24 tickSpacing, address pool)
            if topics.len() < 4 { return None; }
            let token_0 = Address::from_word(topics[1]);
            let token_1 = Address::from_word(topics[2]);
            let fee = u32::from_be_bytes([0, topics[3][29], topics[3][30], topics[3][31]]);
            
            let data = log.data().data.as_ref();
            let pool = Address::from_slice(data.get(44..64)?);

            return Some(NewPoolEvent::V3(V3PoolData {
                pool, token_0, token_1, fee, dex_name: *dex_name,
            }));
        }

        None
    }
}
