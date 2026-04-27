use crate::models::{MEVError, Chain};
use alloy::providers::{Provider, RootProvider};
use alloy::transports::BoxTransport;
use alloy::network::{TransactionBuilder, EthereumWallet};
use alloy::signers::local::PrivateKeySigner as LocalWallet;
use alloy::rpc::types::TransactionRequest;
use alloy_primitives::{Address, U256};
use std::sync::Arc;
use tracing::{info, debug, error};
use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use crate::nonce_manager::NonceManager;
use crate::utils::L1DataFeeCalculator;

pub struct ProfitManager {
    provider: Arc<RootProvider<BoxTransport>>,
    wallet: LocalWallet,
    nonce_manager: Arc<NonceManager>,
    l1_calc: Arc<L1DataFeeCalculator>,
    chain: Chain,
    gas_reserve_threshold: U256,
    compounding_target: Option<Address>,
    total_profit_harvested: Arc<AtomicU64>,
    daily_profit: Arc<AtomicU64>,
    running: AtomicBool,
}

impl ProfitManager {
    pub fn new(
        provider: Arc<RootProvider<BoxTransport>>,
        wallet: LocalWallet,
        nonce_manager: Arc<NonceManager>,
        l1_calc: Arc<L1DataFeeCalculator>,
        chain: Chain,
        gas_reserve_threshold: U256,
        compounding_target: Option<Address>,
    ) -> Self {
        Self {
            provider,
            wallet,
            nonce_manager,
            l1_calc,
            chain,
            gas_reserve_threshold,
            compounding_target,
            total_profit_harvested: Arc::new(AtomicU64::new(0)),
            daily_profit: Arc::new(AtomicU64::new(0)),
            running: AtomicBool::new(false),
        }
    }

    pub async fn report_harvest(&self) {
        let daily = self.daily_profit.swap(0, Ordering::SeqCst) as f64 / 1e9;
        let total = self.total_profit_harvested.load(Ordering::SeqCst) as f64 / 1e9;
        info!("🌾 [PILLAR P] DAILY HARVEST REPORT - Today: {:.6} ETH | Total Vault: {:.6} ETH", daily, total);
    }

    pub async fn handle_profit(&self, profit_amount: U256) -> Result<(), MEVError> {
        // Pillar P: Prevent concurrent sweep attempts during high-frequency wins
        if self.running.swap(true, Ordering::SeqCst) { return Ok(()); }

        info!("💰 [PILLAR P] New profit captured: {} wei", profit_amount);
        let profit_gwei = (profit_amount / U256::from(1_000_000_000u64)).to::<u64>();
        self.daily_profit.fetch_add(profit_gwei, Ordering::Relaxed);
        self.total_profit_harvested.fetch_add(profit_gwei, Ordering::Relaxed);

        let wallet_addr = self.wallet.address();
        let current_balance: U256 = self.provider
            .get_balance(wallet_addr)
            .await
            .map_err(|e| MEVError::Other(e.to_string()))?;

        // Pillar P: Strategic L1+L2 Fee Awareness
        let l1_fee = self.l1_calc.estimate_l1_fee(self.chain, &[]).await.unwrap_or(U256::ZERO);
        let gas_price = self.provider.get_gas_price().await.unwrap_or(50_000_000u128); // Bug Fix: expected u128
        let total_transfer_cost = l1_fee + (U256::from(21_000) * U256::from(gas_price));

        // Survival Rule: Ensure we keep enough for at least 5 more arbitrage attempts
        let survival_threshold = self.gas_reserve_threshold + (total_transfer_cost * U256::from(5));

        let excess = current_balance.saturating_sub(survival_threshold);
        
        // Only sweep if excess is meaningful ( > 0.005 ETH ) to justify transfer gas
        if excess < U256::from(5 * 10u128.pow(15)) {
            debug!("🛡️ [PILLAR P] Balance {:.6} ETH below sweep threshold. Retaining.", current_balance.to::<u128>() as f64 / 1e18);
            self.running.store(false, Ordering::SeqCst);
            return Ok(());
        }

        // Pillar P: Gas Vault & Compounding Automation
        let mut distribution_balance = excess;
        
        let wallet = EthereumWallet::from(self.wallet.clone());
        let provider = alloy::providers::ProviderBuilder::new()
            .wallet(wallet)
            .on_provider(self.provider.clone());

        if let Some(vault_addr) = crate::constants::GAS_VAULT_ADDRESS {
            let vault_share = (excess * U256::from(crate::constants::GAS_VAULT_PERCENTAGE)) / U256::from(100);
            if vault_share > total_transfer_cost {
                info!("🛡️ [PILLAR P] Sending {} wei to Gas Vault {:?}", vault_share, vault_addr);
                let tx = TransactionRequest::default()
                    .with_to(vault_addr)
                    .with_from(wallet_addr)
                    .with_value(vault_share)
                    .with_nonce(self.nonce_manager.next());
                
                match provider.send_transaction(tx).await {
                    Ok(pending) => { 
                        let _ = pending.watch().await;
                        distribution_balance = distribution_balance.saturating_sub(vault_share); 
                    }
                    Err(e) => error!("❌ [PILLAR P] Vault transfer failed: {}", e),
                }
            }
        }

        if let Some(target) = self.compounding_target {
            if distribution_balance > total_transfer_cost {
                info!("📈 [PILLAR P] Compounding {} wei to {:?}", distribution_balance, target);
                let tx = TransactionRequest::default()
                    .with_to(target)
                    .with_from(wallet_addr)
                    .with_value(distribution_balance)
                    .with_nonce(self.nonce_manager.next());
                
                if let Ok(pending) = provider.send_transaction(tx).await {
                    let _ = pending.watch().await;
                }
            }
        }

        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }
}
