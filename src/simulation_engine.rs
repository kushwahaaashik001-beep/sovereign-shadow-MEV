use revm::{
    db::CacheDB,
    primitives::{Address, TransactTo, U256, AccountInfo, Bytecode, ExecutionResult, Env},
    Evm,
};
use alloy_primitives::Bytes;
use crate::models::MEVError;
use crate::state_mirror::StateMirror;
use std::sync::Arc;

pub struct SimulationEngine {
    db: CacheDB<Arc<StateMirror>>,
}

impl SimulationEngine {
    pub fn new(mirror: Arc<StateMirror>) -> Self {
        // Pillar S: Local Cache layer on top of StateMirror
        let db = CacheDB::new(mirror);
        Self { db }
    }

    /// Simulates the arbitrage transaction locally using REVM
    pub async fn simulate_execution(
        &mut self,
        bot_address: Address,
        calldata: Vec<u8>,
        target_block_number: u64,
    ) -> Result<(U256, u64), MEVError> {
        // Setup Environment locally
        let mut env = Env::default();
        env.block.number = U256::from(target_block_number);
        env.tx.caller = bot_address;
        env.tx.transact_to = TransactTo::Call(bot_address);
        env.tx.data = Bytes::from(calldata).into();
        env.tx.gas_limit = 1_000_000;
        env.tx.gas_price = U256::from(1_000_000_000); // 1 Gwei

        let mut evm = Evm::builder()
            .with_db(&mut self.db)
            .with_env(Box::new(env))
            .build();

        // Execute
        let bal_before = self.db.basic_ref(bot_address)
            .map(|opt| opt.map(|a| a.balance).unwrap_or_default())
            .unwrap_or_default();

        let res = evm.transact_commit().map_err(|e| MEVError::SimulationFailed(e.to_string()))?;
        
        match res {
            ExecutionResult::Success { gas_used, .. } => {
                let bal_after = self.db.basic_ref(bot_address)
                    .map(|opt| opt.map(|a| a.balance).unwrap_or_default())
                    .unwrap_or_default();
                let profit = bal_after.saturating_sub(bal_before);
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
}