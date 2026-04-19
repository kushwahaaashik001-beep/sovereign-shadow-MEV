#![allow(dead_code)]
use alloy_primitives::U256;
use alloy::providers::Provider;
use crate::models::Chain;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;
use futures_util::StreamExt;
use std::collections::VecDeque;

pub struct GasPriceFeed {
    pub base_fee: Arc<watch::Receiver<U256>>,
    pub priority_fee: Arc<watch::Receiver<U256>>,
    pub l1_fee: Arc<watch::Receiver<U256>>,
    priority_history: Arc<Mutex<VecDeque<U256>>>,
}

impl GasPriceFeed {
    pub async fn new(ws_provider_pool: Arc<crate::WsProviderPool>, _chain: Chain) -> Self {
        let (base_tx, base_rx) = watch::channel(U256::ZERO);
        let (priority_tx, priority_rx) = watch::channel(U256::ZERO);
        let (l1_tx, l1_rx) = watch::channel(U256::ZERO);
        let priority_history = Arc::new(Mutex::new(VecDeque::with_capacity(5)));
        
        let history = priority_history.clone();
        let pool = ws_provider_pool.clone();

        tokio::spawn(async move {
            loop {
                let mut block_count = 0;
                // Role: WSS_BLOCKS (Head 0)
                let (_, ws_provider) = pool.get_head(0);
                if let Ok(sub) = ws_provider.subscribe_blocks().await {
                    let mut stream = sub.into_stream();
                    while let Some(block) = stream.next().await {
                        let base_fee = block.header.base_fee_per_gas.unwrap_or_default();
                        let _ = base_tx.send(U256::from(base_fee));
                        
                        // Fetch priority fee estimate
                        if let Ok(priority) = ws_provider.get_max_priority_fee_per_gas().await {
                            let p_u256 = U256::from(priority);
                            let mut h = history.lock().unwrap();
                            if h.len() >= 5 { h.pop_front(); }
                            h.push_back(p_u256);
                            
                            // Calculate median of last 5
                            let mut sorted: Vec<_> = h.iter().cloned().collect();
                            sorted.sort();
                            let median = sorted[sorted.len() / 2];
                            
                            // Adaptive Overshoot: Beat median by 12%
                            let _ = priority_tx.send((median * U256::from(112)) / U256::from(100));
                        }

                        // Pillar P: Throttle L1 Oracle calls (Every 5 blocks)
                        block_count += 1;
                        if block_count % 5 == 0 {
                            let oracle = crate::utils::IGasPriceOracle::IGasPriceOracleInstance::new(
                                crate::constants::OPTIMISM_GAS_ORACLE, 
                                ws_provider.clone()
                            );
                            if let Ok(l1_val) = oracle.l1BaseFee().call().await {
                                let _ = l1_tx.send(l1_val._0);
                            }
                        }
                        
                        // Pillar P: Network Heat Adjustment (Congestion Sleep)
                        if base_fee > (crate::constants::NETWORK_CONGESTION_GWEI * 1_000_000_000) as u128 {
                            tracing::warn!("🔥 [GAS FEED] High Congestion ({} gwei). Sleeping engine...", base_fee / 1_000_000_000);
                            tokio::time::sleep(tokio::time::Duration::from_secs(12)).await;
                        }
                    }
                }
                tracing::error!("🔄 [GAS FEED] Subscription lost. Reconnecting...");
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        });

        Self {
            base_fee: Arc::new(base_rx),
            priority_fee: Arc::new(priority_rx),
            l1_fee: Arc::new(l1_rx),
            priority_history,
        }
    }

    pub async fn current(&self) -> (U256, U256, U256) {
        let base = *self.base_fee.borrow();
        let priority = *self.priority_fee.borrow();
        let l1 = *self.l1_fee.borrow();
        (base, priority, l1)
    }
}
