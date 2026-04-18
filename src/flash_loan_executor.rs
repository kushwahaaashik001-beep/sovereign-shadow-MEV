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
use dashmap::DashMap;

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
    pub occupied_pools:   Arc<DashMap<(u64, Address), String>>, // Pillar SEQ: (Block, Pool) -> OppID
    pub http_pool:        Option<Arc<crate::WsProviderPool>>,  // Pillar L: High-speed validation pool
    vetted_tokens:        Arc<DashMap<Address, bool>>,         // Speed: Skip honeypot for known safe tokens
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
        http_pool:        Option<Arc<crate::WsProviderPool>>,
    ) -> Result<Self, MEVError> {
        let chain_id = provider.get_chain_id().await.map_err(|e| MEVError::Other(e.to_string()))?;
        let chain = Chain::try_from_id(chain_id).unwrap_or(Chain::Base);
        Ok(Self {
            provider, contract_address, min_profit_wei, chain, bundle_builder,
            bribe_percent, nonce_manager, circuit_breaker,
            state_simulator, bidding_engine, l1_calc,
            occupied_pools: Arc::new(DashMap::new()),
            http_pool,
            vetted_tokens: Arc::new(DashMap::new()),
        })
    }

    /// Pillar SEQ: Prune old block entries from the sequencer to save memory.
    pub fn prune_sequencer(&self, current_block: u64) {
        self.occupied_pools.retain(|(block, _), _| *block >= current_block);
    }

    /// Pillar M + N: Simulate locally then execute. Zero-Loss Shield enforced.
    pub async fn simulate_and_execute(&self, opp: &Opportunity) -> Result<B256, MEVError> {
        let builder = self.bundle_builder.as_ref().ok_or(MEVError::Other("BundleBuilder missing".into()))?;
        let mut target_block = builder.block_tracker.current() + 1;

        if self.circuit_breaker.is_open() {
            return Err(MEVError::CircuitBreakerOpen);
        }

        // Pillar SEQ: Bundle Sequencer - Immediate Collision Locking
        for hop in &opp.path.hops {
            if let Some(existing_opp) = self.occupied_pools.get(&(target_block, hop.pool_address)) {
                warn!("⚠️ [SEQUENCER] Collision detected: opp={} uses pool {:?} already occupied by opp={}", 
                    opp.id, hop.pool_address, existing_opp.value());
                return Err(MEVError::Other("Self-collision detected".into()));
            }
        }
        
        // Lock pools early to prevent redundant simulations of overlapping paths
        for hop in &opp.path.hops {
            self.occupied_pools.insert((target_block, hop.pool_address), opp.id.clone());
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
        // HTTP pool ka use karke rapid fire honeypot checks kar rahe hain.
        let mut fetch_tasks = Vec::with_capacity(opp.path.hops.len() * 2);
        let fetch_provider_tuple = self.http_pool.as_ref().map(|p| p.get_head(1)) // Role: HTTP_SIMULATE (Head 1)
            .unwrap_or_else(|| (0, self.provider.clone()));
        let fetch_provider = fetch_provider_tuple.1;

        for hop in &opp.path.hops {
            fetch_tasks.push(self.state_simulator.mirror.fetch_and_cache_bytecode(hop.token_out, fetch_provider.clone()));
            fetch_tasks.push(self.state_simulator.mirror.fetch_and_cache_bytecode(hop.pool_address, fetch_provider.clone()));
        }
        futures::future::join_all(fetch_tasks).await;

        // Execution Speed Optimization: Concurrent Validation & Simulation
        // Honeypot checks are heavy; running them in parallel with simulations saves precious milliseconds.
        let mut validation_tasks = Vec::new();
        for hop in &opp.path.hops {
            if !self.vetted_tokens.contains_key(&hop.token_out) && !crate::constants::CORE_TOKENS.contains(&hop.token_out) {
                let sim = self.state_simulator.clone();
                let token = hop.token_out;
                let pool = hop.pool_address;
                let amount = opp.input_amount;
                validation_tasks.push(tokio::spawn(async move {
                    sim.check_honeypot(token, pool, amount)
                }));
            }
        }

        let mut sim_tasks = Vec::new();
        for lender in [Lender::Balancer, Lender::AaveV3] {
            let calldata = self.build_calldata_with_lender(opp, lender)?;
            let simulator = self.state_simulator.clone();
            let opp_clone = opp.clone();
            let contract = self.contract_address;
            let caller = self.nonce_manager.address;
            
            sim_tasks.push(tokio::spawn(async move {
                let results = simulator.run_branch_simulation(&opp_clone, opp_clone.input_amount, calldata.clone(), contract, caller).await;
                (lender, calldata, results)
            }));
        }

        // Await validations first (fail fast)
        for v_task in validation_tasks {
            if let Ok(Err(e)) = v_task.await {
                return Err(e);
            }
        }
        // Cache successful validations to skip next time
        for hop in &opp.path.hops {
            self.vetted_tokens.insert(hop.token_out, true);
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

        // Budget Protection Rule: Gas cost must be < 10% of total balance.
        // This prevents a single high-gas trade from depleting our survival fund ($3 budget limit).
        let budget_cap = current_balance / U256::from(10);
        if !current_balance.is_zero() && adjusted_total_cost > budget_cap {
            warn!("🛡️ [BUDGET PROTECT] Trade rejected: Gas cost ({}) exceeds 10% of balance (Cap: {})", adjusted_total_cost, budget_cap);
            return Err(MEVError::SimulationFailed("Gas cost exceeds safety budget cap".into()));
        }
        
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

        // Pillar N: The Revert Protection Rule
        // ₹300 Budget Shield: Net_Profit must strictly exceed ALL costs.
        let execution_cost = adjusted_total_cost + bribe_wei;
        if top_sim.profit <= execution_cost {
            let msg = format!("PROTECTION: Profit {} <= Total Cost {} (L1+L2+Bribe). Dropping to save gas budget.", top_sim.profit, execution_cost);
            warn!("🛡️ {}", msg);
            return Err(MEVError::SimulationFailed(msg));
        }

        if !top_sim.success {
            return Err(MEVError::SimulationFailed("Local EVM Revert detected. Dropping bundle.".into()));
        }
        info!("🚀 [EXEC] opp={} Net: {} | L1: {} | L2: {} wei", opp.id, net_profit, l1_fee, l2_execution_fee);

        // Pillar Y: Dynamic Scavenger Filter
        // Instead of a hard $2, we scale based on current network congestion.
        // Survival Mode: Strict Profit Threshold for $3 Budget.
        // We only fire if Net Profit > $0.50 (approx 200,000,000,000,000 wei).
        let min_threshold = U256::from(200_000_000_000_000u128); // $0.50 Minimum Profit

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
        
        let mut attempts = 0;
        let max_resilience_attempts = 2; // Target current block and the next one if it fails

        loop {
            let mut bundle = Bundle::new();
            bundle.target_block = target_block;
            bundle.transactions = vec![signed_rlp.to_vec()];
            bundle.set_bribe(bribe_wei);

            info!("📡 [PILLAR I] Bidding {}% ({} wei) for opp={} | Block={} | Attempt={}/{}", 
                bribe_pct, bribe_wei, opp.id, target_block, attempts + 1, max_resilience_attempts);

            let broadcast_results: Vec<Result<(), String>> = builder.broadcast_bundle(bundle).await;
            let success_count = broadcast_results.iter().filter(|r| r.is_ok()).count();
            
            // Pillar S: Systemic Self-Healing & Calibration
            if success_count > 0 {
                info!("✅ [TX] Bundle accepted by {} relays! Inclusion in block {} expected.", success_count, target_block);
                self.bidding_engine.record_success(opp, bribe_pct);
                self.circuit_breaker.record_success();
                return Ok(tx_hash);
            } else {
                let err_msg = format!("{:?}", broadcast_results);
                
                // Pillar R: Block-Skip Resilience Logic
                // Agar latency ki wajah se block nikal gaya, toh nonce recycle karke turant agle block ke liye try karo.
                let is_stale = err_msg.contains("already passed") || err_msg.contains("stale") || err_msg.contains("expired");
                if is_stale && attempts < max_resilience_attempts - 1 {
                    warn!("⏳ [BLOCK-SKIP] Block {} passed for opp={}. Re-broadcasting for block {}...", target_block, opp.id, target_block + 1);
                    target_block += 1;
                    attempts += 1;
                    continue;
                }

                // Pillar N: Zero-Loss Rejection Analysis
                if err_msg.contains("0x937c4424") || err_msg.to_lowercase().contains("zerolossshield") {
                    warn!("🛡️ [SHIELD] Atomic Revert Protected for opp={}. Gas saved, no loss.", opp.id);
                    self.circuit_breaker.record_failure(FailureType::Slippage);
                } else {
                    error!("❌ [TX] Bundle rejected for opp={}. Msg: {}", opp.id, err_msg);
                    self.circuit_breaker.record_failure(FailureType::Other);
                }
                self.bidding_engine.record_failure(opp);
                return Err(MEVError::NoRelayAccepted);
            }
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
