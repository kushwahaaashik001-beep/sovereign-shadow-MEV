use crate::models::Opportunity;
use crate::state_mirror::StateMirror;
use crate::constants::{KNOWN_COMPETITORS, MIN_BUILDER_TIP_WEI};
use ethers::types::{Chain, U256, H256};
use rustc_hash::FxHashMap;
use std::sync::Arc;
use arc_swap::ArcSwap;
use tracing::{info, debug, warn};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use dashmap::DashMap;
use ethers::types::Address;

/// Tracks block builder performance and winning tips.
pub struct BuilderStats {
    pub builder_name: String,
    pub avg_tip_percent: u32,
    pub inclusion_rate: f64,
}

/// Adaptive Bidding Engine that adjusts bribes based on network heat.
pub struct BiddingEngine {
    stats: ArcSwap<FxHashMap<H256, BuilderStats>>,
    default_bribe: u32,
    state_mirror: Arc<StateMirror>,
    last_success: ArcSwap<Option<(Instant, u32)>>, // Pillar I: Success tracking
    predator_hits: AtomicU64, // Track how many times we countered a predator
    predator_pressure: AtomicU64, // Pillar H: Real-time network heat from competitors
    wins: AtomicU64,
    losses: AtomicU64,
    competitor_history: Arc<DashMap<Address, (AtomicU64, AtomicU64)>>, // [NEW] (Wins, Losses) against address
}

impl BiddingEngine {
    pub fn new(state_mirror: Arc<StateMirror>) -> Self {
        Self {
            stats: ArcSwap::from_pointee(FxHashMap::default()),
            default_bribe: 50, // Pillar I: Survival Mode - Be greedier with profit retention
            state_mirror,
            last_success: ArcSwap::from_pointee(None),
            wins: AtomicU64::new(0),
            losses: AtomicU64::new(0),
            predator_hits: AtomicU64::new(0),
            predator_pressure: AtomicU64::new(0),
            competitor_history: Arc::new(DashMap::new()),
        }
    }

    /// Pillar I: Predator-Counter Detection
    /// Detects if a competitor is trying to front-run or copy our trade.
    pub fn is_predator_active(&self, opp: &Opportunity) -> bool {
        opp.trigger_sender.map_or(false, |s| KNOWN_COMPETITORS.contains(&s))
    }

    /// Calculates the optimal bribe percentage based on opportunity and chain heat.
    /// Pillar I: Adaptive Bidding Logic
    pub fn calculate_bribe(&self, opp: &Opportunity) -> u32 {
        // Pillar H: Predator-Counter Strike
        if self.is_predator_active(opp) {
            info!("⚔️ [PILLAR H] Predator Counter-Run! Bidding for absolute dominance (99%). Target: {:?}", opp.trigger_sender);
            self.predator_hits.fetch_add(1, Ordering::Relaxed);
            
            // Record pressure to escalate bidding for subsequent opportunities in the same block
            self.predator_pressure.fetch_add(1, Ordering::Relaxed);
            
            // [INTELLIGENCE] Check historical win-rate against this specific predator
            if let Some(sender) = opp.trigger_sender {
                if let Some(stats) = self.competitor_history.get(&sender) {
                    let wins = stats.0.load(Ordering::Relaxed) as f64;
                    let losses = stats.1.load(Ordering::Relaxed) as f64;
                    let total = wins + losses;

                    let win_rate = if total > 0.0 { wins / total } else { 0.0 };
                    
                    if win_rate < 0.3 {
                        debug!("🩸 [PILLAR I] Losing to this predator frequently. Forcing 99.9% bribe.");
                        return 99;
                    }
                }
            }

            return 99; // Leave them 1% dust, just to insult them.
        }

        // Pillar I: Internal Win-Rate Analysis
        let wins = self.wins.load(Ordering::Relaxed);
        let losses = self.losses.load(Ordering::Relaxed);
        let total = wins + losses;
        
        let mut win_rate_adjustment = 0i32;
        if total > 5 {
            let win_rate = (wins as f64) / (total as f64);
            if win_rate < 0.4 {
                info!("📉 [BIDDING] Critical Win-Rate ({:.2}%). Escalating bribes.", win_rate * 100.0);
                win_rate_adjustment = 15; // Boost bribe if we are losing
            } else if win_rate > 0.8 {
                win_rate_adjustment = -5; // Lower bribe if we are dominating
            }
        }

        // Pillar H: Environmental Awareness
        // If we've seen multiple predators in this block, boost the base bribe.
        let pressure = self.predator_pressure.load(Ordering::Relaxed);
        if pressure > 2 {
            win_rate_adjustment += 10;
        }

        let base_bribe = match opp.chain {
            // On Mainnet, competition is fierce. We must be aggressive.
            Chain::Mainnet => {
                // Check priority fee from StateMirror to gauge network heat.
                // This is a proxy for how much others are paying for inclusion.
                let priority = self.state_mirror.current_priority_fee();
                
                // If priority fees are very high (> 10 Gwei), it's a bidding war.
                // We must bid a very high percentage to win.
                if priority > U256::from(10_000_000_000u64) { // > 10 gwei
                    info!("🔥 Bidding war detected! Priority fee > 10 Gwei. Using 95% bribe.");
                    95
                // If priority fees are moderate (> 2 Gwei), be competitive.
                } else if priority > U256::from(2_000_000_000u64) { // > 2 gwei
                    85
                } else {
                    // In calm conditions, a lower bribe is fine.
                    75
                }
            }, 
            // On L2s, competition is lower, but speed still matters.
            // A slightly lower base bribe is acceptable.
            Chain::Arbitrum | Chain::Base | Chain::Optimism => 50, // Pillar I: Conservative L2 Bidding
            _ => self.default_bribe,
        };

        // Pillar I: God-Tier Opportunity Logic
        // If profit > 0.1 ETH, we bid for absolute dominance to guarantee the billion-dollar mission.
        if opp.expected_profit > U256::from(10u128.pow(17)) { // > 0.1 ETH
            info!("🔥 [GOD-TIER] High-Alpha Opportunity Detected! Forcing 99% bribe.");
            return 99;
        }

        // Standard Whale Logic
        if opp.expected_profit > U256::from(10u128.pow(18)) { // > 1 ETH
            return 99;
        }

        // If profit is small, we lower the bribe to keep it worth it.
        if opp.expected_profit < U256::from(10u128.pow(15)) { // < 0.001 ETH profit
            return 30;
        }

        // God-Mode: Dynamic floor detection. If we are winning consistently, try to lower bribe.
        let mut final_bribe = (base_bribe as i32 + win_rate_adjustment).max(10) as u32;

        // Pillar I: Feedback Anchor
        if let Some((ts, lb)) = self.last_success.load().as_ref() {
            if ts.elapsed().as_secs() < 60 { // Warm history (last 60s)
                final_bribe = (final_bribe + *lb) / 2;
            }
        }

        final_bribe.min(98).max(10)
    }

    /// Pillar I: Adaptive Priority Bidding
    /// Implements the "1-wei Dominance" rule.
    pub fn suggest_priority_fee(&self, opp: &Opportunity, current_priority: U256) -> U256 {
        let mut suggested = current_priority;

        // Pillar Q: Bootstrap Shield (The ₹200 Rule)
        // If the expected profit is less than 5x the gas cost, we are in "High-Risk" territory.
        // In Bootstrap mode, we prioritize survival over volume.
        let gas_cost_estimate = opp.gas_estimate * (self.state_mirror.current_base_fee() + current_priority);
        if opp.expected_profit < (gas_cost_estimate * 2) {
            debug!("🛡️ [BOOTSTRAP] Margin too thin for ₹200 budget. Throttling bidding aggression.");
        }

        // Pillar I: Win-Rate Scaled Buffer
        let wins = self.wins.load(Ordering::Relaxed);
        let losses = self.losses.load(Ordering::Relaxed);
        let _win_rate = if wins + losses > 0 { wins as f64 / (wins + losses) as f64 } else { 1.0 };

        // Pillar I: Surgical 1-wei Dominance & Win-Rate Escalation
        if let Some(comp_gas) = opp.trigger_gas_price {
            let mut adjustment = U256::from(1);
            
            // [INTELLIGENCE] If we lost to this sender recently, bid 1000 wei more instead of 1
            if let Some(sender) = opp.trigger_sender {
                if let Some(stats) = self.competitor_history.get(&sender) {
                    if stats.1.load(Ordering::Relaxed) > stats.0.load(Ordering::Relaxed) {
                        adjustment = U256::from(1000);
                    }
                }
            }

            // Base L2 specific: If we are on Base, the priority fee is often just a "Sequencer Tip".
            // We only need to beat the competitor's tip.
            if opp.chain == Chain::Base {
                let beat_gas = comp_gas.saturating_add(adjustment);
                
                // [HUNTING MODE] Scaled for ₹200 survival: Max 2.5 Gwei during Whale triggers
                let max_allowed_bid = (opp.expected_profit / (opp.gas_estimate * 10)).min(U256::from(2_500_000_000u64)); 
                
                suggested = beat_gas.min(max_allowed_bid);
                debug!("⚔️ [GOD-MODE] Base L2 Surgical Bid: {} wei (Adjustment: {})", suggested, adjustment);
            } else {
                suggested = comp_gas.saturating_add(adjustment);
            }
        }

        // Pillar R: Congestion Multiplier (The Lag Shield)
        let base_fee = self.state_mirror.current_base_fee();
        
        // Base Dynamic Fee: Scale priority fee with network congestion
        let congestion_multiplier = if base_fee > U256::from(500_000_000u64) { 120 } else { 100 };
        suggested = (suggested * congestion_multiplier) / 100;

        // Pillar R: Shadow Simulation - Inclusion Probability Lock
        // Base Hard Cap: 1 Gwei for ₹200 budget safety.
        if base_fee > U256::from(1_000_000_000u64) && opp.chain == Chain::Base { 
            warn!("🛑 [BUDGET VETO] Base Fee too high (>1 Gwei). Aborting to protect vault.");
            return U256::zero(); 
        }

        suggested.max(U256::from(MIN_BUILDER_TIP_WEI))
    }

    pub fn record_success(&self, opp: &Opportunity, bribe_pct: u32) {
        self.wins.fetch_add(1, Ordering::Relaxed);
        if let Some(sender) = opp.trigger_sender {
            let stats = self.competitor_history.entry(sender).or_insert((AtomicU64::new(0), AtomicU64::new(0)));
            stats.0.fetch_add(1, Ordering::Relaxed);
        }
        self.last_success.store(Arc::new(Some((Instant::now(), bribe_pct))));
    }

    pub fn record_failure(&self, opp: &Opportunity) {
        self.losses.fetch_add(1, Ordering::Relaxed);
        if let Some(sender) = opp.trigger_sender {
            let stats = self.competitor_history.entry(sender).or_insert((AtomicU64::new(0), AtomicU64::new(0)));
            stats.1.fetch_add(1, Ordering::Relaxed);
        }
    }
    
    /// Pillar H: Block reset for pressure
    pub fn reset_pressure(&self) {
        self.predator_pressure.store(0, Ordering::Relaxed);
    }

    /// Updates builder stats from eth_getBundleStats (called periodically).
    pub fn update_stats(&self, new_stats: FxHashMap<H256, BuilderStats>) {
        self.stats.store(Arc::new(new_stats));
    }
}
