use crate::models::{Chain, Opportunity};
use crate::state_mirror::StateMirror;
use crate::constants::{KNOWN_COMPETITORS, MIN_BUILDER_TIP_WEI, MAX_BRIBE_PCT};
use alloy_primitives::{Address, U256, B256};
use std::sync::Arc;
use arc_swap::ArcSwap;
use tracing::warn;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use dashmap::DashMap;
use rustc_hash::FxHashMap;

pub struct BuilderStats {
    pub builder_name: String,
    pub avg_tip_percent: u32,
    pub inclusion_rate: f64,
}

pub struct BiddingEngine {
    stats: ArcSwap<FxHashMap<B256, BuilderStats>>,
    default_bribe: u32,
    pub state_mirror: Arc<StateMirror>,
    last_success: ArcSwap<Option<(Instant, u32)>>, 
    predator_hits: AtomicU64,
    predator_pressure: AtomicU64,
    wins: AtomicU64,
    losses: AtomicU64,
    consecutive_losses: AtomicU64,
    last_heat_score: AtomicU64,
    last_pressure_update: ArcSwap<Instant>,
    competitor_history: Arc<DashMap<Address, (AtomicU64, AtomicU64)>>,
}

impl BiddingEngine {
    pub fn new(state_mirror: Arc<StateMirror>) -> Self {
        Self {
            stats: ArcSwap::from_pointee(FxHashMap::default()),
            default_bribe: 50,
            state_mirror,
            last_success: ArcSwap::from_pointee(None),
            wins: AtomicU64::new(0),
            losses: AtomicU64::new(0),
            consecutive_losses: AtomicU64::new(0),
            predator_hits: AtomicU64::new(0),
            last_pressure_update: ArcSwap::from_pointee(Instant::now()),
            last_heat_score: AtomicU64::new(0),
            predator_pressure: AtomicU64::new(0),
            competitor_history: Arc::new(DashMap::new()),
        }
    }

    pub fn is_predator_active(&self, opp: &Opportunity) -> bool {
        opp.trigger_sender.map_or(false, |s| KNOWN_COMPETITORS.contains(&s))
    }

    pub fn calculate_bribe(&self, opp: &Opportunity) -> u32 {
        // Pillar I: Dynamic Pressure Decay (Nanosecond Calibration)
        let now = Instant::now();
        let last_update = **self.last_pressure_update.load();
        if now.duration_since(last_update).as_secs() > 60 {
            // Slowly bleed off pressure if no predators seen in last 60s
            let p = self.predator_pressure.load(Ordering::Relaxed);
            self.predator_pressure.store(p.saturating_sub(1), Ordering::Relaxed);
            self.last_pressure_update.store(Arc::new(now));
        }

        // AI Alpha Hunter: Aggressive Bidding for high-volatility AI tokens
        // If the path contains AI stars, we go for 95% bribe to guarantee inclusion.
        let mut path_contains_ai = false;
        for hop in &opp.path.hops {
            if hop.token_in == crate::constants::TOKEN_VIRTUAL || hop.token_out == crate::constants::TOKEN_VIRTUAL ||
               hop.token_in == crate::constants::TOKEN_LUNA || hop.token_out == crate::constants::TOKEN_LUNA ||
               hop.token_in == crate::constants::TOKEN_AI16Z || hop.token_out == crate::constants::TOKEN_AI16Z {
                path_contains_ai = true;
                break;
            }
        }

        if path_contains_ai {
            return 95.min(MAX_BRIBE_PCT);
        }

        // 1. Predator Detection (Pillar H)
        if self.is_predator_active(opp) {
            self.predator_hits.fetch_add(1, Ordering::Relaxed);
            self.predator_pressure.fetch_add(1, Ordering::Relaxed);
            self.last_pressure_update.store(Arc::new(now));
            
            // Mafia Logic: If predator is a frequent winner, don't play around.
            if let Some(sender) = opp.trigger_sender {
                if let Some(stats) = self.competitor_history.get(&sender) {
                    let (wins, losses) = (stats.0.load(Ordering::Relaxed), stats.1.load(Ordering::Relaxed));
                    if wins > losses { return 98.min(MAX_BRIBE_PCT); }
                }
            }
            
            return 92.min(MAX_BRIBE_PCT); // High competition floor
        }

        // 2. Whale Trigger Adjustment
        // Increased competition expected for transactions exceeding 10 ETH (Whale alerts).
        let is_whale = opp.is_whale_trigger;

        // 3. Adaptive profit-based tiers from constants
        let profit_val = opp.expected_profit.to::<u128>();
        let mut bribe_pct = self.default_bribe;

        // Pillar Y: Micro-Profit Shield
        // If the profit is tiny (Scavenger Mode), keep the bribe minimal to cover gas.
        if profit_val < crate::constants::MICRO_PROFIT_THRESHOLD_WEI {
            return 5; // Bare minimum bribe for micro-arbs
        }
        
        for (threshold, pct) in crate::constants::BIDDING_TIERS.iter() {
            if profit_val >= *threshold {
                bribe_pct = *pct as u32;
            } else {
                break;
            }
        }

        // 4. Failure-Rate Awareness: If we are on a losing streak, ramp up bribes.
        let loss_streak = self.consecutive_losses.load(Ordering::Relaxed);
        if loss_streak > 2 {
            bribe_pct = (bribe_pct + (loss_streak as u32 * 8)).min(MAX_BRIBE_PCT - 9);
        }

        // 5. Pillar L: Mafia Logic (Derivative of Heat)
        // Combine BaseFee volatility and Predator pressure with higher weights for L2.
        let current_heat = (self.predator_pressure.load(Ordering::Relaxed) * 15) + 
                          (self.state_mirror.current_base_fee().to::<u128>() / 1_000_000_000) as u64;
        
        let prev_heat = self.last_heat_score.swap(current_heat, Ordering::SeqCst);
        let heat_derivative = if prev_heat > 0 { (current_heat as f64 / prev_heat as f64) - 1.0 } else { 0.0 };

        // Mafia Mode: Heat rising rapidly (>15%) OR a heavy loss streak triggers total aggression.
        if heat_derivative > 0.15 || loss_streak > 5 {
            bribe_pct = bribe_pct.max(94); // Mafia Mode: "Win at all costs"
        }

        if is_whale {
            bribe_pct = (bribe_pct + 10).min(MAX_BRIBE_PCT);
        }

        if let Some((ts, lb)) = self.last_success.load().as_ref() {
            if ts.elapsed().as_secs() < 60 {
                bribe_pct = (bribe_pct + *lb) / 2;
            }
        }
        bribe_pct.min(MAX_BRIBE_PCT).max(10)
    }

    pub fn suggest_priority_fee(&self, opp: &Opportunity, current_priority: U256) -> U256 {
        let mut suggested = current_priority;

        let is_mafia = self.last_heat_score.load(Ordering::Relaxed) > 50 || self.consecutive_losses.load(Ordering::Relaxed) > 3;

        // Strict Gas Price Cap: 0.1 gwei (100,000,000 wei)
        let base_fee = self.state_mirror.current_base_fee();
        if base_fee > U256::from(100_000_000u64) {
            return U256::ZERO; // Abort: Gas too expensive for $3 budget
        }

        if let Some(comp_gas) = opp.trigger_gas_price {
            // Adaptive overshoot: Beat competitors by 2 gwei (4 gwei in Mafia Mode)
            let adjustment = if is_mafia { U256::from(4_000_000_000u64) } else { U256::from(2_000_000_000u64) };
            if opp.chain == Chain::Base {
                // Zero-Loss Shield: Ensure fees don't consume more than 50% of expected profit
                let profit_limit_factor = if is_mafia { U256::from(1u64) } else { U256::from(2u64) }; 
                let max_allowed = opp.expected_profit / (opp.gas_estimate.max(U256::from(1u64)) * profit_limit_factor);
                suggested = comp_gas.saturating_add(adjustment).min(max_allowed.into());
            } else {
                suggested = comp_gas.saturating_add(adjustment);
            }
        }

        let base_fee = self.state_mirror.current_base_fee();
        let congestion_multiplier = if base_fee > U256::from(500_000_000u64) { 120u64 } else { 100u64 };
        suggested = (suggested * U256::from(congestion_multiplier)) / U256::from(100u64);

        if base_fee > U256::from(1_000_000_000u64) && opp.chain == Chain::Base {
            warn!("🛑 [BUDGET VETO] Base Fee too high. Aborting.");
            return U256::ZERO;
        }

        suggested.max(U256::from(MIN_BUILDER_TIP_WEI))
    }

    pub fn record_success(&self, opp: &Opportunity, bribe_pct: u32) {
        self.wins.fetch_add(1, Ordering::Relaxed);
        self.consecutive_losses.store(0, Ordering::Relaxed); // Reset streak on win
        if let Some(sender) = opp.trigger_sender {
            let stats = self.competitor_history.entry(sender).or_insert((AtomicU64::new(0), AtomicU64::new(0)));
            stats.0.fetch_add(1, Ordering::Relaxed);
        }
        self.last_success.store(Arc::new(Some((Instant::now(), bribe_pct))));
    }

    pub fn record_failure(&self, opp: &Opportunity) {
        self.losses.fetch_add(1, Ordering::Relaxed);
        self.consecutive_losses.fetch_add(1, Ordering::Relaxed);
        if let Some(sender) = opp.trigger_sender {
            let stats = self.competitor_history.entry(sender).or_insert((AtomicU64::new(0), AtomicU64::new(0)));
            stats.1.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn reset_pressure(&self) {
        self.predator_pressure.store(0, Ordering::Relaxed);
    }

    pub fn update_stats(&self, new_stats: FxHashMap<B256, BuilderStats>) {
        self.stats.store(Arc::new(new_stats));
    }
}
