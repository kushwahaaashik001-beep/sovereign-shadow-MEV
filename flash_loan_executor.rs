use alloy::providers::{RootProvider, Provider};
use alloy::network::{TransactionBuilder, EthereumWallet};
use alloy::eips::eip2718::Encodable2718;
use alloy::rpc::types::TransactionRequest;
use alloy::sol;
use alloy::sol_types::SolCall;
use alloy::transports::BoxTransport;
use alloy_primitives::{Address, U256, B256};
use std::sync::Arc;
use tracing::{info, warn, error, debug};

use crate::bidding_engine::BiddingEngine;
use crate::bundle_builder::BundleBuilder;
use crate::models::{Chain, MEVError, Opportunity, Lender, Bundle};
use crate::nonce_manager::NonceManager;
use crate::state_simulator::StateSimulator;
use crate::utils::{CircuitBreaker, FailureType, L1DataFeeCalculator};

use crate::state_simulator::SimResult; // Import SimResult

sol! {
    #[sol(rpc)]
    interface IShadowBot {
        function executeArbitrage(
            bytes calldata pathData
        ) external;
    }
}

/// Pillar M: Flash loan executor — borrows capital, executes arb, repays in one tx.
pub struct FlashLoanExecutor {
    pub provider:         Arc<RootProvider<BoxTransport>>,
    pub contract_address: Address,
    pub min_profit_wei:   U256,
    pub chain:            Chain,
    pub bundle_builder:   Option<Arc<BundleBuilder>>,
    pub bribe_percent:    u64,
    nonce_manager:        Arc<NonceManager>,
    circuit_breaker:      Arc<CircuitBreaker>,
    state_simulator:      Arc<StateSimulator>,
    bidding_engine:       Arc<BiddingEngine>,
    l1_calc:              Arc<L1DataFeeCalculator>,
}

impl FlashLoanExecutor {
    pub async fn new(
        provider:         Arc<RootProvider<BoxTransport>>,
        contract_address: Address,
        min_profit_wei:   U256,
        bundle_builder:   Option<Arc<BundleBuilder>>,
        bribe_percent:    u64,
        nonce_manager:    Arc<NonceManager>,
        circuit_breaker:  Arc<CircuitBreaker>,
        state_simulator:  Arc<StateSimulator>,
        bidding_engine:   Arc<BiddingEngine>,
        l1_calc:          Arc<L1DataFeeCalculator>,
    ) -> Result<Self, MEVError> {
        let chain_id = provider.get_chain_id().await.map_err(|e| MEVError::Other(e.to_string()))?;
        let chain = Chain::try_from_id(chain_id).unwrap_or(Chain::Base);
        Ok(Self {
            provider, contract_address, min_profit_wei, chain, bundle_builder,
            bribe_percent, nonce_manager, circuit_breaker,
            state_simulator, bidding_engine, l1_calc,
        })
    }

    /// Pillar M + N: Simulate locally then execute. Zero-Loss Shield enforced.
    pub async fn simulate_and_execute(&self, opp: &Opportunity) -> Result<B256, MEVError> {
        if self.circuit_breaker.is_open() {
            return Err(MEVError::CircuitBreakerOpen);
        }

        // Pillar T: Anti-Drift Guardian. 
        // Enforce sub-millisecond state freshness before simulation.
        self.state_simulator.mirror.verify_state_freshness()?;

        // Pillar W: Detect Wash-Trade Traps before committing resources.
        for hop in &opp.path.hops {
            self.state_simulator.detect_wash_trap(hop.pool_address)?;
        }

        // Pillar L: Ensure bytecode is cached and scanned for honeypot patterns before simulation.
        // Static analysis must happen before dynamic simulation for 100% coverage.
        let mut fetch_tasks = Vec::with_capacity(opp.path.hops.len() * 2);
        for hop in &opp.path.hops {
            fetch_tasks.push(self.state_simulator.mirror.fetch_and_cache_bytecode(hop.token_out, self.provider.clone()));
            fetch_tasks.push(self.state_simulator.mirror.fetch_and_cache_bytecode(hop.pool_address, self.provider.clone()));
        }
        futures::future::join_all(fetch_tasks).await;

        // Pillar H: Predator Detection + Pillar L: Honeypot Check
        for hop in &opp.path.hops {
            self.state_simulator.check_honeypot(hop.token_out, hop.pool_address, opp.input_amount)?;
        }

        // Pillar M: Ultra-Fast Parallel Lender Selection (Aave V3 vs Balancer)
        let mut sim_tasks = Vec::new();
        for lender in [Lender::Balancer, Lender::AaveV3] {
            let calldata = self.build_calldata_with_lender(opp, lender)?;
            let simulator = self.state_simulator.clone();
            let opp_clone = opp.clone();
            let contract = self.contract_address;
            let dummy_caller = alloy_primitives::address!("0x000000000000000000000000000000000000dEaD");
            
            sim_tasks.push(tokio::spawn(async move {
                let results = simulator.run_branch_simulation(&opp_clone, opp_clone.input_amount, calldata.clone(), contract, dummy_caller).await;
                (lender, calldata, results)
            }));
        }

        let sim_outputs = futures::future::join_all(sim_tasks).await;
        let mut best_candidate: Option<(Lender, SimResult, Vec<u8>, U256, U256, U256, U256, alloy::rpc::types::AccessList)> = None;

        let (base_fee, priority_fee, _) = self.bidding_engine.state_mirror.gas_state.load().current_fees();
        let total_gas_price = base_fee + priority_fee;

        for task_res in sim_outputs {
            if let Ok((lender, calldata, sim_results)) = task_res {
                if let Some(top_sim) = sim_results.first() {
                    if top_sim.success && top_sim.profit >= self.min_profit_wei {
                        let is_robust = sim_results.iter().skip(1).all(|r| {
                            r.success && r.profit >= self.min_profit_wei && 
                            (top_sim.profit.saturating_sub(r.profit) * U256::from(10000) / top_sim.profit.max(U256::from(1))) <= U256::from(crate::constants::MAX_BRANCH_LOSS_BPS)
                        });

                        if is_robust {
                            let l1_fee = self.l1_calc.estimate_l1_fee(self.chain, &calldata).await.unwrap_or(U256::ZERO);
                            let l2_fee = U256::from(top_sim.gas_used) * total_gas_price;
                            let total_cost = l1_fee + l2_fee;
                            let net_profit = top_sim.profit.saturating_sub(total_cost);

                            // Gas Efficiency Bias: Balancer is usually ~15k gas cheaper at the EVM level.
                            // We add a tiny virtual bonus to Balancer to prefer it if profits are nearly equal.
                            let mut adjusted_net = net_profit;
                            if lender == Lender::Balancer {
                                adjusted_net += U256::from(50_000_000_000_000u128); // 0.00005 ETH bias
                            }

                            if best_candidate.as_ref().map_or(true, |(_, _, _, _, _, _, prev_adj, _)| adjusted_net > *prev_adj) {
                                best_candidate = Some((lender, top_sim.clone(), calldata, net_profit, l1_fee, l2_fee, adjusted_net, top_sim.access_list.clone()));
                            }
                        }
                    }
                }
            }
        }

        // Pillar M: Selection Finalization
        let (lender, top_sim, calldata, net_profit, l1_fee, l2_execution_fee, _, best_access_list) = 
            best_candidate.ok_or(MEVError::SimulationFailed("No lender provided profitable/robust results".into()))?;

        let _total_cost = l1_fee + l2_execution_fee;

        debug!("💎 [OPTIMIZER] Selected {:?} | Gas Used: {} | Net: {} wei", lender, top_sim.gas_used, net_profit);

        // Pillar S: The Ironclad Survival Rule + Pillar Y: Yield Scavenger
        // Dynamic adjustment: If we have a surplus of gas, we scavenge micro-profits.
        let l1_buffer = (l1_fee * U256::from(120)) / U256::from(100); // 20% Extra L1 safety
        let adjusted_total_cost = l1_buffer + l2_execution_fee;
        
        let current_balance = self.circuit_breaker.get_cached_balance();
        let scavenger_threshold = U256::from(50_000_000_000_000_000u128); // 0.05 ETH
        
        let _survival_multiplier = if current_balance > scavenger_threshold {
            // Pillar Y: Yield Scavenging Mode - Aggressive scrap collection
            U256::from(12) // 1.2x
        } else {
            U256::from(30) // 3.0x Standard Survival
        };

        // Pillar I: Adaptive Bribe
        let bribe_pct = self.bidding_engine.calculate_bribe(opp);
        let bribe_wei = (net_profit * U256::from(bribe_pct)) / U256::from(100u64);

        // Hybrid Efficiency Logic: Strict check for Net_Profit > Gas_Cost + Bribe
        let execution_cost = adjusted_total_cost + bribe_wei;
        if top_sim.profit <= execution_cost {
            return Err(MEVError::SimulationFailed(format!(
                "Unprofitable: Profit {} <= Total Cost {}", top_sim.profit, execution_cost
            )));
        }

        if !top_sim.success {
            return Err(MEVError::SimulationFailed("Local EVM Revert detected. Dropping bundle.".into()));
        }
        info!("🚀 [EXEC] opp={} Net: {} | L1: {} | L2: {} wei", opp.id, net_profit, l1_fee, l2_execution_fee);

        // Pillar Y: Dynamic Scavenger Filter
        // Instead of a hard $2, we scale based on current network congestion.
        // If gas is low, we take smaller, high-frequency profits to reach the $100/day goal.
        let current_gas_price = total_gas_price;
        let min_threshold = if current_gas_price < U256::from(500_000_000u64) { // Gas < 0.5 gwei
            U256::from(400_000_000_000_000u128) // $1.00 minimum
        } else {
            U256::from(crate::constants::MIN_NET_PROFIT_USD_WEI) // $2.00 standard
        };

        if net_profit < min_threshold {
            return Err(MEVError::SimulationFailed(format!(
                "Scavenger Veto: Net profit {} < dynamic threshold", net_profit
            )));
        }

        // Pillar I: Calculate Adaptive Bribe
        let mut bribe_pct = self.bidding_engine.calculate_bribe(opp);

        // Competitive Bumping: If a known predator is in the mempool, bump bribe by 10%
        if opp.trigger_sender.map_or(false, |f| crate::constants::KNOWN_COMPETITORS.contains(&f)) {
            bribe_pct = std::cmp::min(crate::constants::MAX_BRIBE_PCT, bribe_pct + 10);
            info!("⚠️ [PREDATOR] Competitor detected! Bumping bribe to {}%", bribe_pct);
        }

        let bribe_wei = (net_profit * U256::from(bribe_pct)) / U256::from(100u64);

        let builder = self.bundle_builder.as_ref().ok_or(MEVError::Other("BundleBuilder missing".into()))?;
        let signer = &builder.config.signer; // Field made public in bundle_builder.rs

        // Pillar I: Calculate Ultra-Precise Gas for Submission
        let suggested_priority = self.bidding_engine.suggest_priority_fee(opp, priority_fee);
        let max_fee = base_fee + suggested_priority;

        // Pillar M + R: Build signed RLP with EIP-2930 Access List optimization
        let tx_request = TransactionRequest::default()
            .with_to(self.contract_address)
            .with_from(signer.address())
            .with_input(calldata)
            .with_nonce(self.nonce_manager.next())
            .with_access_list(best_access_list) // Shadow Simulation injection
            .with_gas_limit(((top_sim.gas_used as f64 * 1.1) as u64 + 10_000) as u128) // 10% buffer + 10k fixed is safer
            .with_max_fee_per_gas(max_fee.to::<u128>())
            .with_max_priority_fee_per_gas(suggested_priority.to::<u128>())
            .with_chain_id(builder.config.chain_id); // Field made public in bundle_builder.rs

        let wallet = EthereumWallet::from(signer.clone());
        let signed_tx = tx_request.build(&wallet).await
            .map_err(|e| MEVError::Other(format!("FlashLoan Signing Failed: {}", e)))?;
        
        let signed_rlp = signed_tx.encoded_2718(); // Requires Encodable2718 trait
        let tx_hash = *signed_tx.tx_hash(); // Bug Fix: use tx_hash()
        
        let mut bundle = Bundle::new();
        bundle.target_block = builder.block_tracker.current() + 1;
        bundle.transactions = vec![signed_rlp];
        bundle.set_bribe(bribe_wei);

        info!("📡 [PILLAR I] Bidding {}% ({} wei) for opp={} | TxHash: {:?}", bribe_pct, bribe_wei, opp.id, tx_hash);

        // Pillar S: Multi-Relay Strategic Submission
        // Base builders need ultra-low latency. We broadcast to all relays in parallel 
        // and monitor for "Flashbots-Specific" rejection codes to self-correct.
        let builder_clone = builder.clone();
        let broadcast_results: Vec<Result<(), String>> = builder_clone.broadcast_bundle(bundle).await;
        
        let success_count = broadcast_results.iter().filter(|r| r.is_ok()).count();
        
        info!("📡 [RELAY] Attempted to broadcast bundle to {} relays: {:?}", builder.config.relays.len(), builder.config.relays);
        // Pillar S: Systemic Self-Healing & Calibration
        if success_count > 0 {
            info!("✅ [TX] Bundle accepted by {} relays! Recording success.", success_count);
            self.bidding_engine.record_success(opp, bribe_pct);
            self.circuit_breaker.record_success();
            Ok(tx_hash)
        } else {
            // Pillar N: Zero-Loss Rejection Analysis
            let err_msg = format!("{:?}", broadcast_results);
            if err_msg.contains("0x937c4424") || err_msg.to_lowercase().contains("zerolossshield") {
                warn!("🛡️ [SHIELD] On-chain Revert Protected for opp={}. Gas saved, no loss.", opp.id);
                self.circuit_breaker.record_failure(FailureType::Slippage);
            } else if err_msg.contains("nonce too low") {
                warn!("🔄 [NONCE] Collision detected on relay. Forcing nonce refresh...");
                self.nonce_manager.refresh().await;
            } else {
                error!("❌ [TX] Relay Blackout: All {} relays rejected. Check block tracker.", broadcast_results.len());
                self.circuit_breaker.record_failure(FailureType::Other);
            }
            self.bidding_engine.record_failure(opp);
            Err(MEVError::NoRelayAccepted)
        }
    }

    fn build_calldata_with_lender(&self, opp: &Opportunity, lender: Lender) -> Result<Vec<u8>, MEVError> {
        let loans = vec![(opp.input_token, opp.input_amount)]; 
        let path = crate::models::ArbitragePath {
            hops:        opp.path.hops.clone(),
            loans:       loans.clone(),
            lender,
        };

        // Bug Fix: Construct proper ABI encoded call for ShadowBot.sol
        let path_data = path.encode_ghost_multi(loans, self.min_profit_wei);
        
        let call = IShadowBot::executeArbitrageCall {
            pathData:   path_data,
        };

        Ok(call.abi_encode())
    }
}
