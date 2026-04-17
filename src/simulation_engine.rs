use revm::{
    db::CacheDB,
    primitives::{Address, TransactTo, U256, AccountInfo, Bytecode, ExecutionResult, Env, TxEnv},
    Evm,
};
use alloy_primitives::Bytes;
use crate::models::{MEVError, Chain};
use crate::state_mirror::StateMirror;
use std::sync::Arc;
use tracing::{debug, warn};

pub struct SimulationEngine {
    db: CacheDB<Arc<StateMirror>>,
    pub base_asset: Address,
}

impl SimulationEngine {
    pub fn new(mirror: Arc<StateMirror>, chain: Chain) -> Self {
        // Pillar S: Local Cache layer on top of StateMirror
        let db = CacheDB::new(mirror);
        let base_asset = if chain == Chain::Base {
            alloy_primitives::address!("4200000000000000000000000000000000000006") // WETH
        } else {
            Address::ZERO
        };
        Self { db, base_asset }
    }

    /// Simulates the arbitrage transaction locally using REVM
    pub async fn simulate_execution(
        &mut self,
        bot_address: Address,
        calldata: Vec<u8>,
        target_block_number: u64,
        min_profit_threshold: U256,
    ) -> Result<(U256, u64), MEVError> {
        // Setup Environment locally
        let mut env = Env::default();
        env.block.number = U256::from(target_block_number);
        env.tx.caller = bot_address;
        env.tx.transact_to = TransactTo::Call(bot_address);
        env.tx.data = Bytes::from(calldata).into();
        env.tx.gas_limit = crate::constants::SIMULATION_GAS_LIMIT;
        env.tx.gas_price = U256::from(1_000_000_000); // 1 Gwei

        let mut evm = Evm::builder()
            .with_db(&mut self.db)
            .with_env(Box::new(env))
            .build();

        // Pillar L: Balance Check - Initial state check
        // Optimized: Directly call the helper without rebuilding the EVM
        let bal_before = self.get_balance_optimized(&mut evm, self.base_asset, bot_address)?;

        // Execute and commit state to self.db automatically
        let res = evm.transact_commit().map_err(|e| MEVError::SimulationFailed(e.to_string()))?;
        
        match res {
            ExecutionResult::Success { gas_used, .. } => {
                // Pillar L: Post-execution check
                // Optimized: We use the same EVM instance. 
                // We don't need a new builder because transact_commit already updated the DB.
                let bal_after = self.get_balance_optimized(&mut evm, self.base_asset, bot_address)?;
                let profit = bal_after.saturating_sub(bal_before);

                // Pillar N: Zero-Loss Shield (Internal Verification)
                if profit < min_profit_threshold {
                    debug!("⚠️ [SIM] Profit {} below threshold {}", profit, min_profit_threshold);
                    return Err(MEVError::SimulationFailed("Insufficient profit after execution".into()));
                }

                // Pillar L: Honeypot/Tax Protection
                // Agar profit expected se 90% kam hai, matlab raste mein tax ya honeypot tha.
                if profit.is_zero() && !min_profit_threshold.is_zero() {
                    warn!("🚨 [SIM] Honeypot Detected: Successful TX but zero liquid balance increase!");
                    return Err(MEVError::HoneypotDetected("Zero liquid profit in simulation".into()));
                }

                Ok((profit, gas_used))
            }
            _ => Err(MEVError::SimulationFailed("Local EVM Revert".into())),
        }
    }

    pub fn inject_state(&mut self, address: Address, code: Vec<u8>, balance: U256) {
        let info = AccountInfo {
            balance,
            nonce: 0,
            code_hash: revm::primitives::keccak256(&code),
            code: Some(Bytecode::new_raw(Bytes::from(code))),
        };
        self.db.insert_account_info(address, info);
    }

    /// Ultra-optimized balance fetcher. Avoids cloning and environment overhead.
    fn get_balance_optimized<DB: revm::Database>(&self, evm: &mut Evm<'_, (), DB>, token: Address, account: Address) -> Result<U256, MEVError> 
    where <DB as revm::Database>::Error: std::fmt::Debug
    {
        let mut data = vec![0x70, 0xa0, 0x82, 0x31];
        data.extend_from_slice(&[0u8; 12]);
        data.extend_from_slice(account.as_slice());

        // Set temporary environment for static call without cloning original
        let prev_data = std::mem::replace(&mut evm.context.evm.env.tx.data, Bytes::from(data).into());
        let prev_to = std::mem::replace(&mut evm.context.evm.env.tx.transact_to, TransactTo::Call(token));
        
        let res = evm.transact().map_err(|e| MEVError::SimulationFailed(format!("{:?}", e)))?.result;

        // Fast restore
        evm.context.evm.env.tx.data = prev_data;
        evm.context.evm.env.tx.transact_to = prev_to;

        if let ExecutionResult::Success { output, .. } = res {
            Ok(U256::from_be_slice(output.data()))
        } else {
            Ok(U256::ZERO)
        }
    }
}