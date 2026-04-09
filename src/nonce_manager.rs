use crate::models::MEVError;
use ethers::prelude::*;
use ethers::providers::Middleware;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone)]
pub struct NonceManager<M: Middleware> {
    current: Arc<AtomicU64>,
    provider: Arc<M>,
    address: Address,
}

impl<M: Middleware + 'static> NonceManager<M> {
    pub async fn new(provider: Arc<M>, address: Address) -> Result<Self, MEVError> {
        let nonce = provider
            .get_transaction_count(address, None)
            .await
            .map_err(|_| MEVError::Other("nonce fetch failed".into()))?
            .as_u64();
        Ok(Self { current: Arc::new(AtomicU64::new(nonce)), provider, address })
    }

    /// Sync atomic increment — nanosecond speed, no await needed in hot path.
    #[inline(always)]
    pub fn next(&self) -> u64 {
        self.current.fetch_add(1, Ordering::SeqCst)
    }

    /// Async version for callers that use `?` operator.
    pub async fn next_nonce(&self) -> Result<u64, MEVError> {
        Ok(self.next())
    }

    /// Re-sync from chain on stuck/dropped tx.
    pub async fn resync(&self) -> Result<(), MEVError> {
        let on_chain = self.provider
            .get_transaction_count(self.address, None)
            .await
            .map_err(|_| MEVError::Other("resync failed".into()))?
            .as_u64();
        self.current.store(on_chain, Ordering::SeqCst);
        Ok(())
    }
}
