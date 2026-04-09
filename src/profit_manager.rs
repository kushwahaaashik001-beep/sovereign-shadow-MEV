// Pillar P: The Auto-Compounding Vault
use crate::models::MEVError;
use ethers::types::{TransactionRequest, transaction::eip2718::TypedTransaction};
use ethers::prelude::*;
use std::sync::Arc;
use tracing::{info, warn, debug};
use std::sync::atomic::AtomicU64;
use crate::utils::send_telegram_msg;

pub struct ProfitManager<M: Middleware> {
    provider: Arc<M>,
    wallet: LocalWallet,
    gas_reserve_threshold: U256,
    // The address to send compounded profits to. If None, they accumulate in the bot wallet.
    compounding_target: Option<Address>,
    _last_harvest_time: Arc<AtomicU64>,
}

impl<M: Middleware + 'static> ProfitManager<M> {
    pub fn new(
        provider: Arc<M>,
        wallet: LocalWallet,
        gas_reserve_threshold: U256,
        compounding_target: Option<Address>,
    ) -> Self {
        Self {
            provider,
            wallet,
            gas_reserve_threshold,
            compounding_target,
            _last_harvest_time: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Manages profits from a successful trade.
    /// It ensures the gas reserve is met, and sends excess profits to the compounding target.
    pub async fn handle_profit(&self, profit_amount: U256) -> Result<(), MEVError> {
        info!("💰 [PILLAR P] New profit captured: {} wei", profit_amount);

        let current_balance = self.provider.get_balance(self.wallet.address(), None).await.map_err(|e| MEVError::Other(e.to_string()))?;
        let balance_eth = current_balance.as_u128() as f64 / 1e18;

        // Autonomous Reporting
        let profit_eth = profit_amount.as_u128() as f64 / 1e18;
        let msg = format!(
            "👑 *Sovereign Shadow Alpha Found!*\n\n💰 Profit: `{:.6} ETH`\n🏦 Wallet Balance: `{:.6} ETH`", 
            profit_eth, balance_eth
        );
        send_telegram_msg(&msg);
        
        let total_capital = current_balance; 

        // Pillar Q: Maintenance Threshold ($30 / 0.01 ETH)
        // Isse upar ka saara balance automatically harvest ho jayega.
        let excess_profit = total_capital.saturating_sub(self.gas_reserve_threshold);

        if excess_profit.is_zero() {
            debug!("🛡️ [PILLAR P] Balance is below $30 threshold. Retaining for gas.");
            return Ok(());
        }

        // Brilliant Idea: Gas-Theft Protection
        let gas_price = self.provider.get_gas_price().await.unwrap_or_default();
        let transfer_cost = gas_price * U256::from(21_000u64);
        
        if excess_profit <= transfer_cost {
            debug!("🛡️ [PILLAR P] Excess ({}) too small. Retaining to avoid 'Gas Theft' (Cost: {}).", excess_profit, transfer_cost);
            return Ok(());
        }

        // [100% SWEEP] Calculate exact amount to send leaving exactly 0.01 ETH
        // We subtract transfer_cost manually to ensure the transaction doesn't fail due to gas.
        let amount_to_compound = excess_profit.saturating_sub(transfer_cost);

        if let Some(target_address) = self.compounding_target {
            info!("[PILLAR P] Harvesting Excess: {} wei to Cold Vault {}", amount_to_compound, target_address);

            let mut tx: TypedTransaction = TransactionRequest::new()
                .to(target_address)
                .value(amount_to_compound)
                .from(self.wallet.address())
                .into();

            // Fill, Sign, and Send Raw
            self.provider.fill_transaction(&mut tx, None).await.map_err(|e| MEVError::Other(e.to_string()))?;
            let signature = self.wallet.sign_transaction(&tx).await.map_err(MEVError::WalletError)?;
            let raw_tx = tx.rlp_signed(&signature);

            match self.provider.send_raw_transaction(raw_tx).await {
                Ok(pending_tx) => {
                    info!("[Pillar P] Compounding transaction sent: {:?}", pending_tx.tx_hash());
                    // In production, you might want to await confirmation.
                }
                Err(e) => {
                    warn!("[Pillar P] Failed to send compounding transaction: {}", e);
                    // If the tx fails, the funds remain in the wallet, which is safe.
                }
            }
        } else {
            info!("[Pillar P] {} wei designated for compounding, but no target address set. Profit remains in wallet.", amount_to_compound);
        }

        Ok(())
    }
}