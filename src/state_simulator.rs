use crate::models::{Opportunity, MEVError};
use crate::state_mirror::StateMirror;
use ethers::types::{Address, Bytes, U256};
use std::sync::Arc;
use revm::{
    db::{CacheDB, EmptyDB},
    primitives::{
        Address as rAddress, Bytecode, Bytes as rBytes,
        ExecutionResult, Output, TransactTo, U256 as rU256,
        AccountInfo, keccak256,
    },
    EVM,
};

// Architect Tip: Ensure that the REVM environment is configured with 
// the exact L2 block context (timestamp, difficulty) to avoid 
// "Multiverse Drift" where the simulation passes but the real TX fails.

pub struct StateSimulator {
    mirror: Arc<StateMirror>,
    executor_address: Address,
}

impl StateSimulator {
    pub fn new(mirror: Arc<StateMirror>, executor_address: Address) -> Self {
        Self {
            mirror,
            executor_address,
        }
    }

    pub fn get_cached_gas(&self) -> U256 {
        self.mirror.current_base_fee()
    }

    pub fn get_next_base_fee(&self) -> U256 {
        self.mirror.gas_state.load().next_base_fee
    }

    /// Pillar K: Multiverse Drift Protection Simulation
    /// Runs a local simulation to verify profitability before execution.
    pub fn simulate_multiverse(
        &self,
        opp: &Opportunity,
        calldata: &Bytes,
        ghost: Option<(Address, Vec<u8>)>,
    ) -> Result<(U256, u64), MEVError> {
        // Pillar T: Ensure state freshness before allowing success
        self.mirror.verify_state_freshness()?;

        // [PILLAR B] God-Mode REVM Integration
        let mut db = CacheDB::new(EmptyDB::default());

        // 1. Load Executor Bytecode into simulation DB
        let executor_r = rAddress::from_slice(self.executor_address.as_bytes());
        let executor_code = self.mirror.get_bytecode(&self.executor_address)
            .ok_or_else(|| MEVError::Other("Executor bytecode missing from StateMirror cache".into()))?;
        
        db.insert_account_info(executor_r, AccountInfo {
            balance: rU256::ZERO,
            nonce: 0,
            code_hash: keccak256(executor_code.bytes()),
            code: Some(executor_code),
        });

        // 2. Handle Ghost Protocol Injection (Ephemeral Minimal Proxy)
        let target_r = if let Some((ghost_addr, ghost_code)) = ghost {
            let ghost_r = rAddress::from_slice(ghost_addr.as_bytes());
            let bc = Bytecode::new_raw(ghost_code.into());
            db.insert_account_info(ghost_r, AccountInfo {
                balance: rU256::ZERO,
                nonce: 0,
                code_hash: keccak256(bc.bytes()),
                code: Some(bc),
            });
            ghost_r
        } else {
            executor_r
        };

        // 3. Configure EVM Instance with Block Context
        let mut evm = EVM::new();
        evm.database(db);

        // Set L2 Context (Pillar O)
        evm.env.block.number = rU256::from(self.mirror.current_block_number());
        evm.env.block.basefee = rU256::from_limbs(self.mirror.current_base_fee().0);
        evm.env.tx.caller = rAddress::from_slice(&[0x1; 20]); // Dummy searcher address
        evm.env.tx.transact_to = TransactTo::Call(target_r);
        evm.env.tx.data = rBytes::from(calldata.0.to_vec());
        evm.env.tx.gas_limit = opp.gas_estimate.as_u64();

        // 4. Fire Simulation
        let result = evm.transact().map_err(|e| MEVError::SimulationFailed(format!("REVM Execution Error: {:?}", e)))?;

        match result.result {
            ExecutionResult::Success { output, gas_used, .. } => {
                let profit = match output {
                    Output::Call(data) if data.len() >= 32 => {
                        U256::from_big_endian(&data[0..32])
                    }
                    _ => U256::zero(),
                };

                // Zero-Profit Veto: If simulation doesn't match math, something changed.
                if profit.is_zero() {
                    return Err(MEVError::SimulationFailed("Simulation returned 0 profit. Slippage or state shift detected.".into()));
                }

                Ok((profit, gas_used))
            }
            ExecutionResult::Revert { output, .. } => {
                Err(MEVError::SimulationFailed(format!("Simulation REVERTED: {:?}", output)))
            }
            ExecutionResult::Halt { reason, .. } => {
                Err(MEVError::SimulationFailed(format!("Simulation HALTED: {:?}", reason)))
            }
        }
    }

    pub fn simulate_arbitrage(&self, opp: &Opportunity, calldata: &[u8], ghost: Option<(Address, Vec<u8>)>) -> Result<(U256, u64), MEVError> {
        self.simulate_multiverse(opp, &Bytes::from(calldata.to_vec()), ghost)
    }
}