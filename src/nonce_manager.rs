use alloy::providers::{Provider, RootProvider};
use alloy::transports::BoxTransport;
use alloy_primitives::Address;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::models::MEVError;

/// Atomic nonce manager — zero-latency increment, async resync on failure.
#[derive(Clone)]
pub struct NonceManager {
    current:  Arc<AtomicU64>,
    provider: Arc<RootProvider<BoxTransport>>,
    address:  Address,
}

impl NonceManager {
    pub async fn new(provider: Arc<RootProvider<BoxTransport>>, address: Address) -> Result<Self, MEVError> {
        let nonce = provider
            .get_transaction_count(address)
            .await
            .map_err(|e| MEVError::Other(e.to_string()))?;
        Ok(Self { current: Arc::new(AtomicU64::new(nonce)), provider, address })
    }

    #[inline(always)]
    pub fn next(&self) -> u64 { self.current.fetch_add(1, Ordering::SeqCst) }

    pub async fn resync(&self) -> Result<(), MEVError> {
        let n = self.provider
            .get_transaction_count(self.address)
            .await
            .map_err(|e| MEVError::Other(e.to_string()))?;
        self.current.store(n, Ordering::SeqCst);
        Ok(())
    }
}
