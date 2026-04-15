#![allow(dead_code)]
use alloy::providers::{ProviderBuilder, RootProvider};
use alloy::transports::BoxTransport;
use std::sync::{atomic::{AtomicUsize, Ordering}, Arc};

pub type HttpProvider = RootProvider<BoxTransport>;

pub struct RpcManager {
    providers: Vec<Arc<HttpProvider>>,
    next: AtomicUsize,
}

impl RpcManager {
    pub fn new(urls: Vec<String>) -> Self {
        let providers = urls.iter()
            .filter_map(|u| u.parse().ok().map(|url| {
                Arc::new(ProviderBuilder::new().on_http(url).boxed())
            }))
            .collect();
        Self { providers, next: AtomicUsize::new(0) }
    }

    pub fn get_next_provider(&self) -> Arc<HttpProvider> {
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.providers.len().max(1);
        self.providers[idx].clone()
    }
}
