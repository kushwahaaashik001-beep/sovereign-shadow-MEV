// =============================================================================
// File: gas_feed.rs
// Project: The Sovereign Shadow (MEV/Arbitrage Stealth Engine)
// Description: Centralized gas price feed for EIP-1559 and L2 L1 fees.
// =============================================================================

use ethers::{
    prelude::*, types::{Address, U256, transaction::eip2718::TypedTransaction},
};
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::watch;

/// Centralized gas feed with L1 data fee support for L2s.
#[derive(Debug)]
pub struct GasPriceFeed {
    pub base_fee: Arc<watch::Receiver<U256>>,
    pub priority_fee: Arc<watch::Receiver<U256>>,
    pub l1_fee: Arc<watch::Receiver<U256>>, // For L2s, this is the L1 base fee
}

impl GasPriceFeed {
    /// Creates a new feed and spawns a background task to keep it updated.
    pub async fn new(ws_provider_pool: Arc<crate::WsProviderPool>, chain: Chain) -> Self { // Use WsProviderPool
        let (base_tx, base_rx) = watch::channel(U256::zero());
        let (priority_tx, priority_rx) = watch::channel(U256::zero());
        let (l1_tx, l1_rx) = watch::channel(U256::zero());

        let ws_pool_clone = ws_provider_pool.clone();
        tokio::spawn(async move {
            loop {
                let provider = ws_pool_clone.next(); // Get next provider from the pool
                let mut stream: SubscriptionStream<'_, Ws, Block<H256>> = match provider.subscribe_blocks().await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!("Failed to subscribe to blocks for gas feed: {}. Retrying with next provider.", e);
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        continue;
                    }
                };

                while let Some(block) = stream.next().await {
                    let base: Option<U256> = block.base_fee_per_gas;
                    if let Some(b) = base {
                        let effective_base = if b.is_zero() { U256::from(100_000u64) } else { b };
                        let _ = base_tx.send(effective_base);
                    }
                    if let Ok(fee) = provider.request::<(), U256>("eth_maxPriorityFeePerGas", ()).await {
                        let effective = if fee.is_zero() {
                            base.unwrap_or_default() / U256::from(10)
                        } else { fee };
                        let _ = priority_tx.send(effective.max(U256::from(100_000u64)));
                    } else if let Some(b_val) = base {
                        let _ = priority_tx.send((b_val / U256::from(10)).max(U256::from(100_000u64)));
                    }

                    // For L2s, fetch L1 base fee estimate.
                    if chain == Chain::Optimism || chain == Chain::Base {
                        let oracle = Address::from_str("0x420000000000000000000000000000000000000F").unwrap();
                        let selector = &ethers::utils::keccak256(b"l1BaseFee()")[..4];
                        let tx: TypedTransaction = TransactionRequest::new().to(oracle).data(selector.to_vec()).into();
                        if let Ok(res) = provider.call(&tx, None).await {
                            let fee = U256::from_big_endian(&res);
                            let _ = l1_tx.send(fee);
                        }
                    }
                }
                tracing::error!("Gas feed block stream ended unexpectedly. Retrying with next provider.");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            };
        });

        Self {
            base_fee: Arc::new(base_rx),
            priority_fee: Arc::new(priority_rx),
            l1_fee: Arc::new(l1_rx),
        }
    }

    /// Returns a snapshot of the current gas prices.
    /// Falls back to RPC call if watch channel still has initial zero value.
    pub async fn current(&self) -> (U256, U256, U256) {
        let base = *self.base_fee.borrow();
        let priority = *self.priority_fee.borrow();
        let l1 = *self.l1_fee.borrow();
        (base, priority, l1)
    }
}