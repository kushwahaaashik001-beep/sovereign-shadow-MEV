#![allow(dead_code)]
use std::sync::Arc;
use alloy_primitives::{Address, U256, Bytes};
use revm::{
    db::CacheDB,
    primitives::{ExecutionResult, TransactTo, TxEnv, AccountInfo, SpecId},
    Evm, DatabaseCommit, 
};
use crate::state_mirror::StateMirror;
use crate::models::{Opportunity, MEVError};

#[derive(Debug, Clone, Copy)]
pub enum SimulationBranch {
    Top,    // Immediately after target tx
    Mid,    // After target + 3 high-priority txs
    Tail,   // At the end of the block
}

#[derive(Debug, Clone)]
pub struct SimResult {
    pub profit: U256,
    pub gas_used: u64,
    pub branch: SimulationBranch,
    pub success: bool,
    pub access_list: alloy::rpc::types::AccessList,
}

pub struct StateSimulator {
    pub mirror: Arc<StateMirror>,
}

impl StateSimulator {
    pub fn new(mirror: Arc<StateMirror>) -> Self {
        Self { mirror }
    }

    /// Pillar K: Branching simulation to verify trade robustness.
    pub async fn run_branch_simulation(
        &self,
        opp: &Opportunity,
        input_amount: U256,
        target_tx_data: Vec<u8>,
        target_contract: Address,
        caller_eoa: Address,
    ) -> Vec<SimResult> {
        let mut results = Vec::new();
        let branches = [SimulationBranch::Top, SimulationBranch::Mid, SimulationBranch::Tail];

        for branch in branches {
            if let Ok(res) = self.simulate_strategy(opp, input_amount, &target_tx_data, target_contract, branch, caller_eoa) {
                results.push(res);
            }
        }
        results
    }

    /// Helper for legacy calls in the engine.
    pub fn simulate_multiverse(
        &self,
        opp: &Opportunity,
        arb_data: &[u8],
        target_contract: Address,
        caller_eoa: Address,
    ) -> Result<(U256, u64, bool), MEVError> {
        let res = self.simulate_strategy(opp, opp.input_amount, arb_data, target_contract, SimulationBranch::Top, caller_eoa)?;
        Ok((res.profit, res.gas_used, res.success))
    }

    fn simulate_strategy(
        &self,
        opp: &Opportunity,
        _input: U256,
        arb_data: &[u8],
        target_contract: Address,
        branch: SimulationBranch,
        caller_eoa: Address,
    ) -> Result<SimResult, MEVError> {
        // Pillar K: Zero-Copy Reference Passing
        // Wrap the existing StateMirror in CacheDB to avoid upfront cloning.
        let mut cache_db = CacheDB::new(self.mirror.as_ref());

        // Add EOA to DB with some ETH for gas payment
        cache_db.insert_account_info(caller_eoa, AccountInfo {
            balance: U256::from(10u128.pow(18)), // 1 ETH for gas
            ..Default::default()
        });

        // Ensure target contract exists in state
        if cache_db.load_account(target_contract).is_err() {
            cache_db.insert_account_info(target_contract, AccountInfo::default());
        }

        let mut evm = Evm::builder()
            .with_db(cache_db)
            .with_spec_id(SpecId::CANCUN) // Pillar R: Cancun alignment for Base/Mainnet
            .build();

        // 3. Inject Trigger Transactions (The original swaps from mempool)
        for tx in &opp.pending_txs {
            evm.context.evm.env.tx = TxEnv {
                caller: Address::from(rand::random::<[u8; 20]>()),
                transact_to: tx.to.map_or(TransactTo::Call(Address::ZERO), TransactTo::Call),
                data: tx.data.clone().into(),
                value: U256::ZERO,
                ..Default::default()
            };
            let _ = evm.transact_commit();
        }

        // 4. Pillar K: Stealth Branching - Inject noise to test robustness
        self.inject_noise_transactions(&mut evm, opp, branch);

        // Measure balance of the CONTRACT (where profit lands) before arb
        let bal_before = evm.context.evm.db.load_account(target_contract)
            .map(|a| a.info.balance).unwrap_or_default();

        // 5. Execute our Arbitrage Tx
        evm.context.evm.env.tx = TxEnv {
            caller: caller_eoa, 
            transact_to: TransactTo::Call(target_contract), 
            data: arb_data.to_vec().into(),
            gas_limit: crate::constants::SIMULATION_GAS_LIMIT,
            ..Default::default()
        };

        // Pillar R: Extract simulated results
        let result = evm.transact().map_err(|e| MEVError::SimulationFailed(e.to_string()))?;
        let access_list = alloy::rpc::types::AccessList::default(); // Placeholder: REVM requires a separate tracer for AL generation
        
        // Commit state manually after extracting info
        evm.context.evm.db.commit(result.state);
        let ref_res = result.result;
        
        let bal_after = evm.context.evm.db.load_account(target_contract)
            .map(|a| a.info.balance).unwrap_or_default();

        let profit = bal_after.saturating_sub(bal_before);
        
        let (success, gas_used) = match ref_res {
            ExecutionResult::Success { gas_used, .. } => (true, gas_used),
            _ => (false, 0),
        };

        Ok(SimResult {
            profit,
            gas_used,
            branch,
            success,
            access_list: access_list.into(), // Convert to alloy-compatible AccessList
        })
    }

    /// Pillar W: Detects volume manipulation traps (Wash Trading).
    /// Blocks execution if the pool lacks organic trading diversity.
    pub fn detect_wash_trap(&self, pool: Address) -> Result<(), MEVError> {
        if let Some(traders_ref) = self.mirror.trader_registry.get(&pool) {
            let unique_count = traders_ref.value().len();
            if unique_count > 0 && unique_count < crate::constants::MIN_UNIQUE_TRADERS {
                // If the pool has active swaps but very few participants, it's likely a wash trap.
                return Err(MEVError::HoneypotDetected(format!(
                    "WASH_TRAP: Pool {:?} has only {} unique traders", 
                    pool, unique_count
                )));
            }
        }
        Ok(())
    }

    /// Pillar L: Poison Token Filter - Dynamic Honeypot & Tax Detection.
    pub fn check_honeypot(
        &self,
        token: Address,
        pool: Address,
        amount_in_wei: U256,
    ) -> Result<u64, MEVError> {
        // Pillar L: Poison Token Filter
        if self.mirror.is_poisoned(&token) || self.mirror.is_poisoned(&pool) {
            return Err(MEVError::HoneypotDetected("Static analysis flagged this pool/token".into()));
        }

        // 1. Manual Blacklist Check
        if let Some(blacklist) = crate::constants::BLACKLISTED_TOKENS.get(&crate::models::Chain::Base) {
            if blacklist.contains(&token) {
                return Err(MEVError::HoneypotDetected(format!("Token {:?} is manually blacklisted", token)));
            }
        }

        // 2. Dynamic Simulation (Pillar L)
        // Use mirror as backend to access token/pool state and bytecode
        let mut cache_db = CacheDB::new(self.mirror.as_ref());
        let sim_executor = Address::from(rand::random::<[u8; 20]>());
        let recipient = Address::from(rand::random::<[u8; 20]>());

        // Give ETH for gas
        cache_db.insert_account_info(sim_executor, AccountInfo {
            balance: U256::from(10u128.pow(18)),
            ..Default::default()
        });

        let mut evm = Evm::builder().with_db(cache_db).build();

        // Step 1: "Buy" tokens by transferring from pool (simulates buy phase)
        let mut buy_data = vec![0xa9, 0x05, 0x9c, 0xbb]; // transfer(address,uint256)
        buy_data.extend_from_slice(&[0u8; 12]);
        buy_data.extend_from_slice(sim_executor.as_slice());
        let mut amt_bytes = [0u8; 32];
        amount_in_wei.to_be_bytes::<32>().copy_from_slice(&mut amt_bytes);
        buy_data.extend_from_slice(&amt_bytes);

        evm.context.evm.env.tx = TxEnv {
            caller: pool,
            transact_to: TransactTo::Call(token),
            data: Bytes::from(buy_data),
            ..Default::default()
        };
        
        let buy_res = evm.transact_commit().map_err(|e| MEVError::SimulationFailed(format!("BUY_EXEC_ERROR: {}", e)))?;
        if !buy_res.is_success() {
             return Err(MEVError::HoneypotDetected("BUY_FAILED: Restricted token distribution".into()));
        }

        // Step 1.1: Verification - Did we actually get the tokens?
        let mut check_data = vec![0x70, 0xa0, 0x82, 0x31];
        check_data.extend_from_slice(&[0u8; 12]);
        check_data.extend_from_slice(sim_executor.as_slice());

        evm.context.evm.env.tx = TxEnv {
            caller: sim_executor,
            transact_to: TransactTo::Call(token),
            data: Bytes::from(check_data),
            ..Default::default()
        };
        let check_res = evm.transact().map_err(|e| MEVError::SimulationFailed(e.to_string()))?.result;
        if let ExecutionResult::Success { output, .. } = check_res {
            let bal = U256::from_be_slice(output.data());
            if bal.is_zero() {
                return Err(MEVError::HoneypotDetected("GHOST_TOKEN: Transfer reported success but balance is 0".into()));
            }
        }

        // Step 1.2: Advance Check - Can we Approve? (Common Honeypot Vector)
        let mut approve_data = vec![0x09, 0x5e, 0xa7, 0xb3]; // approve(address,uint256)
        approve_data.extend_from_slice(&[0u8; 12]);
        approve_data.extend_from_slice(recipient.as_slice());
        approve_data.extend_from_slice(&amt_bytes);

        evm.context.evm.env.tx = TxEnv {
            caller: sim_executor,
            transact_to: TransactTo::Call(token),
            data: Bytes::from(approve_data),
            ..Default::default()
        };
        let app_res = evm.transact_commit().map_err(|e| MEVError::SimulationFailed(e.to_string()))?;
        if !app_res.is_success() {
             return Err(MEVError::HoneypotDetected("APPROVE_FAILED: Token blocks approval logic".into()));
        }

        // Step 2: "Sell" tokens & Gas Trap Detection
        let mut sell_data = vec![0xa9, 0x05, 0x9c, 0xbb];
        sell_data.extend_from_slice(&[0u8; 12]);
        sell_data.extend_from_slice(recipient.as_slice());
        sell_data.extend_from_slice(&amt_bytes);

        evm.context.evm.env.tx = TxEnv {
            caller: sim_executor,
            transact_to: TransactTo::Call(token),
            data: Bytes::from(sell_data),
            ..Default::default()
        };

        let sell_res = evm.transact_commit().map_err(|e| MEVError::SimulationFailed(format!("SELL_EXEC_ERROR: {}", e)))?;
        if !sell_res.is_success() {
            return Err(MEVError::HoneypotDetected("SELL_FAILED: Honeypot detected (revert on transfer)".into()));
        }

        // Gas Trap Check: Simple transfer should not exceed 200k gas
        if let ExecutionResult::Success { gas_used, .. } = sell_res {
            if gas_used > 200_000 {
                return Err(MEVError::HoneypotDetected(format!("GAS_TRAP: Abnormal gas usage ({})", gas_used)));
            }
        }

        // Step 3: Check recipient balance to calculate tax
        let mut bal_data = vec![0x70, 0xa0, 0x82, 0x31];
        bal_data.extend_from_slice(&[0u8; 12]);
        bal_data.extend_from_slice(recipient.as_slice());

        evm.context.evm.env.tx = TxEnv {
            caller: sim_executor,
            transact_to: TransactTo::Call(token),
            data: Bytes::from(bal_data),
            ..Default::default()
        };

        let bal_res = evm.transact().map_err(|e| MEVError::SimulationFailed(e.to_string()))?.result;
        if let ExecutionResult::Success { output, .. } = bal_res {
            let balance_received = U256::from_be_slice(output.data());
            if balance_received < amount_in_wei {
                let tax_bps = (amount_in_wei.saturating_sub(balance_received) * U256::from(10000)) / amount_in_wei;
                let tax_u64 = tax_bps.to::<u64>();
                if tax_u64 > crate::constants::MAX_ALLOWED_TAX_BPS {
                    return Err(MEVError::HoneypotDetected(format!("HIGH_TAX: {} BPS", tax_u64)));
                }
                return Ok(tax_u64);
            }
        }

        Ok(0)
    }

    /// Pillar K: Inject "Stealth Noise" to simulate competing private bundles.
    /// If our trade fails here, it means it's too fragile for the real network.
    fn inject_noise_transactions<DB: revm::Database + DatabaseCommit>(
        &self, 
        evm: &mut Evm<'_, (), DB>, 
        opp: &Opportunity,
        branch: SimulationBranch
    ) {
        let intensity = match branch {
            SimulationBranch::Top => return, // No noise for top-of-block
            SimulationBranch::Mid => 3,      // 3 realistic trades per hop
            SimulationBranch::Tail => 8,     // Heavy block congestion
        };

        // Pillar K: Realistic Noise Selectors for V2 and V3
        let v2_swap_selector = [0x02, 0x2c, 0x0d, 0x9f]; // swap(uint256,uint256,address,bytes)
        let v3_swap_selector = [0x12, 0x8a, 0xa6, 0x9d]; // swap(address,bool,int256,uint160,bytes)

        for i in 0..intensity {
            for hop in &opp.path.hops {
                let mut noise_data = Vec::with_capacity(228);
                let simulate_v3 = i % 2 == 0; // Jitter between V2 and V3 noise

                if simulate_v3 {
                    // Pillar K: High-Intensity V3 Whale Noise (Properly Encoded)
                    // Simulates a retail or predator whale shifting the tick range.
                    noise_data.extend_from_slice(&v3_swap_selector);
                    
                    // recipient (32 bytes)
                    noise_data.extend_from_slice(&[0u8; 12]);
                    noise_data.extend_from_slice(&rand::random::<[u8; 20]>());
                    
                    // zeroForOne (32 bytes)
                    let zfo = if i % 3 == 0 { 1u8 } else { 0u8 };
                    noise_data.extend_from_slice(&[0u8; 31]);
                    noise_data.push(zfo);
                    
                    // amountSpecified (32 bytes - Random 0.5 to 2.5 ETH Whale)
                    let amount = U256::from(10u128.pow(17) * ((i % 20) as u128 + 5));
                    let amt_bytes = amount.to_be_bytes::<32>();
                    noise_data.extend_from_slice(&amt_bytes);
                    
                    // sqrtPriceLimitX96 (32 bytes - Min/Max based on zfo)
                    let sqrt_limit = if zfo == 1 { 
                        U256::from(4295128739u64 + 1) 
                    } else { 
                        U256::from_limbs([0xffffffffffffffff, 0xffffffffffffffff, 0xffffffffffffffff, 0x00000000000003ff])
                    };
                    let sl = sqrt_limit.to_be_bytes::<32>();
                    noise_data.extend_from_slice(&sl);
                    
                    noise_data.extend_from_slice(&[0u8; 31]); noise_data.push(0xa0); // data offset (160)
                    noise_data.extend_from_slice(&[0u8; 32]); // data length (0)
                } else {
                    // Pillar K: Realistic V2 Whale Noise (1.0 to 10.0 ETH)
                    // Drastically shifts reserves to test if our arb survives reserve depletion.
                    noise_data.extend_from_slice(&v2_swap_selector);
                    
                    let amount_out = U256::from(10u128.pow(18) * ((i % 10) as u128 + 1));
                    let mut b = [0u8; 32]; // Fix: Use to_be_bytes()
                    amount_out.to_be_bytes::<32>().copy_from_slice(&mut b);

                    if i % 2 == 0 {
                        noise_data.extend_from_slice(&b); // amount0Out
                        noise_data.extend_from_slice(&[0u8; 32]); // amount1Out
                    } else {
                        noise_data.extend_from_slice(&[0u8; 32]);
                        noise_data.extend_from_slice(&b); // amount1Out
                    }
                    noise_data.extend_from_slice(&[0u8; 12]); // address padding
                    noise_data.extend_from_slice(&rand::random::<[u8; 20]>()); // to
                    noise_data.extend_from_slice(&[0u8; 64]); // data offset + len
                }

                evm.context.evm.env.tx = TxEnv {
                    caller: Address::from(rand::random::<[u8; 20]>()),
                    transact_to: TransactTo::Call(hop.pool_address),
                    data: Bytes::from(noise_data),
                    gas_limit: 350_000, // Higher limit for realistic V3 execution
                    ..Default::default()
                };
                // commit() updates the state permanently for the rest of this simulation branch
                let _ = evm.transact_commit();
            }
        }
    }
}