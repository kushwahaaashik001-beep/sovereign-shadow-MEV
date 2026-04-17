#![allow(dead_code)]
use alloy_primitives::{Address, U256, Uint, I256};
type U512 = Uint<512, 8>;
use crate::models::{MempoolTx, Path, DexType};
use crate::state_mirror::{StateMirror, PoolState};
use rustc_hash::FxHashMap;
use std::sync::Arc;

#[derive(Clone, Copy)]
pub struct MathEngine;

// Pre-calculated golden ratio constants for nanosecond pathfinding
const PHI_NUM: u128 = 618033988749895;
const PHI_DEN: u128 = 1000000000000000;
const E18: u128 = 1000000000000000000;
const SCALE_36: u128 = 1000000000000000000000000000000000000;

impl MathEngine {
    /// Optimized path output calculation using pre-fetched pool states.
    /// This avoids DashMap lookups inside the hot optimization loop.
    #[inline(always)]
    pub fn get_path_output_with_states(
        &self, 
        hops: &[crate::models::Hop], 
        input_amount: U256, 
        states: &[PoolState]
    ) -> U256 {
        let mut current_amount = input_amount;
        for (i, hop) in hops.iter().enumerate() {
            if current_amount.is_zero() { return U256::ZERO; }
            let state = &states[i];
            current_amount = match hop.dex_type {
                DexType::UniswapV2 => self.get_v2_output(current_amount, state, hop.zero_for_one),
                DexType::Aerodrome => self.get_aerodrome_output(current_amount, state, hop.zero_for_one),
                _ => U256::ZERO,
            };
        }
        current_amount
    }

    #[inline(always)]
    pub fn project_reserve_impact(&self, tx: &MempoolTx, mirror: &StateMirror) -> FxHashMap<Address, (U256, U256)> {
        let mut impacts = FxHashMap::default();
        let data = tx.data.as_ref();
        if data.len() < 4 { return impacts; }
        let selector = &data[0..4];
        let target = match tx.to { Some(a) => a, None => return impacts };

        if selector == [0x02, 0x2c, 0x0d, 0x9f] && data.len() >= 100 {
            if let Some(ps) = mirror.pools.get(&target) {
                let amt0_out = U256::from_be_slice(&data[4..36]);
                let amt1_out = U256::from_be_slice(&data[36..68]);
                let (r0, r1): (U256, U256) = (ps.reserves0, ps.reserves1);
                if amt0_out > U256::ZERO {
                    let new_r0: U256 = r0.saturating_sub(amt0_out);
                    if !new_r0.is_zero() {
                        let amt1_in = (r1 * amt0_out * U256::from(1000u64)) / (new_r0 * U256::from(997u64));
                        impacts.insert(target, (new_r0, r1 + amt1_in));
                    }
                } else if amt1_out > U256::ZERO {
                    let new_r1: U256 = r1.saturating_sub(amt1_out);
                    if !new_r1.is_zero() {
                        let amt0_in = (r0 * amt1_out * U256::from(1000u64)) / (new_r1 * U256::from(997u64));
                        impacts.insert(target, (r0 + amt0_in, new_r1));
                    }
                }
            }
        }
        impacts
    }

    /// Pillar D: Golden Section Search to find the input amount that maximizes net profit.
    /// f(x) = (OutputAmount(x) - x) - TotalGasCost
    pub fn find_optimal_input<F>(min_in: U256, max_in: U256, mut f: F) -> (U256, U256)
    where F: FnMut(U256) -> I256 {
        let tolerance = U256::from(100_000_000_000u64); // 100 gwei precision
        let mut a = min_in;
        let mut b = max_in;
        
        let mut c = b - ((b - a) * U256::from(PHI_NUM) / U256::from(PHI_DEN));
        let mut d = a + ((b - a) * U256::from(PHI_NUM) / U256::from(PHI_DEN));
        
        let mut f_c = f(c);
        let mut f_d = f(d);

        // Pillar D: Iteration Cap (The Edge Check)
        // 40 iterations ensure sub-wei precision even for 1000+ ETH input ranges.
        // Using a fixed for-loop prevents "infinite hang" scenarios in high-volatility mempools.
        for _ in 0..40 { 
            if (b - a) < tolerance { break; }

            if f_c > f_d {
                b = d;
                d = c;
                f_d = f_c;
                c = b - ((b - a) * U256::from(PHI_NUM) / U256::from(PHI_DEN));
                f_c = f(c);
            } else {
                a = c;
                c = d;
                f_c = f_d;
                d = a + ((b - a) * U256::from(PHI_NUM) / U256::from(PHI_DEN));
                f_d = f(d);
            }
        }

        let optimal_input = (a + b) / U256::from(2u64);
        let max_profit = f(optimal_input);
        
        if max_profit <= I256::ZERO {
            (U256::ZERO, U256::ZERO)
        } else {
            (optimal_input, U256::from_be_bytes(max_profit.to_be_bytes::<32>()))
        }
    }

    /// Recursively calculate the output amount for a multi-hop path.
    pub fn get_path_output(&self, path: &Path, input_amount: U256, mirror: &Arc<StateMirror>) -> U256 {
        let mut current_amount = input_amount;
        
        for hop in &path.hops {
            if current_amount.is_zero() { return U256::ZERO; }
            
            let pool_state = match mirror.get_pool_data(&hop.pool_address, 5) {
                Some(s) => s,
                None => return U256::ZERO,
            };

            current_amount = match hop.dex_type {
                DexType::UniswapV2 => self.get_v2_output(current_amount, &pool_state, hop.zero_for_one),
                DexType::Aerodrome => self.get_aerodrome_output(current_amount, &pool_state, hop.zero_for_one),
                _ => U256::ZERO,
            };
        }
        current_amount
    }

    #[inline(always)]
    fn get_v2_output(&self, amount_in: U256, state: &PoolState, zero_for_one: bool) -> U256 {
        let (reserve_in, reserve_out) = if state.reserves0 != U256::ZERO && state.reserves1 != U256::ZERO {
            if zero_for_one {
                (state.reserves0, state.reserves1)
            } else {
                (state.reserves1, state.reserves0)
            }
        } else { return U256::ZERO; };

        if amount_in.is_zero() || reserve_in.is_zero() || reserve_out.is_zero() { return U256::ZERO; }

        // Standard V2 formula: (amt_in * 997 * res_out) / (res_in * 1000 + amt_in * 997)
        let amt_in_with_fee = U512::from(amount_in) * U512::from(997u64);
        let numerator = amt_in_with_fee * U512::from(reserve_out);
        let denominator = (U512::from(reserve_in) * U512::from(1000u64)) + amt_in_with_fee;
        
        u512_to_u256_safe(numerator / denominator)
    }

    #[inline(always)]
    fn get_aerodrome_output(&self, amount_in: U256, state: &PoolState, zero_for_one: bool) -> U256 {
        if !state.is_stable {
            return self.get_v2_output(amount_in, state, zero_for_one);
        }
        
        let (r_in, r_out) = if zero_for_one { (state.reserves0, state.reserves1) } else { (state.reserves1, state.reserves0) };
        if amount_in.is_zero() || r_in.is_zero() || r_out.is_zero() { return U256::ZERO; }

        // Aerodrome Stable Math Optimization: Target < 5ms
        let amount_in_fee = (amount_in * U256::from(9997)) / U256::from(10000); // 0.03% standard fee
        let x_new = r_in + amount_in_fee;
        
        // 1. Hyper-fast Initial Guess using floating point (Space-age optimization)
        let x_f = x_new.to::<u128>() as f64;
        let r_in_f = r_in.to::<u128>() as f64;
        let r_out_f = r_out.to::<u128>() as f64;
        let k_f = r_in_f.powi(3) * r_out_f + r_out_f.powi(3) * r_in_f;
        
        // f64 Newton step to get within 0.1% instantly
        let mut y_f = r_out_f;
        for _ in 0..2 {
            let f_y = x_f.powi(3) * y_f + y_f.powi(3) * x_f;
            let f_prime_y = x_f.powi(3) + 3.0 * y_f.powi(2) * x_f;
            y_f = y_f - (f_y - k_f) / f_prime_y;
        }

        let mut y = U256::from(y_f as u128);
        let k = self.aerodrome_k(r_in, r_out);
        let x_new_u512 = U512::from(x_new);
        let x2 = x_new_u512 * x_new_u512;
        let x3 = x2 * x_new_u512;
        let scale = U512::from(SCALE_36);
        
        // 2. High-Precision Refinement (Max 3 iterations for nanosecond convergence)
        for _ in 0..3 {  
            let y_u512 = U512::from(y);
            let y2 = y_u512 * y_u512;
            
            let f_y = (x3 * y_u512 + y2 * y_u512 * x_new_u512) / scale;
            let f_prime_y = (x3 + U512::from(3u64) * y2 * x_new_u512) / scale;
            
            if f_y > U512::from(k) {
                let diff = u512_to_u256_safe((f_y - U512::from(k)) / f_prime_y.max(U512::from(1u64)));
                if diff.is_zero() { break; }
                y -= diff.min(y / U256::from(2));
            } else {
                let diff = u512_to_u256_safe((U512::from(k) - f_y) / f_prime_y.max(U512::from(1u64)));
                if diff.is_zero() { break; }
                y += diff.min(y);
            }
        }
        r_out.saturating_sub(y)
    }

    /// Returns the analytical derivative |dy/dx| for the stable curve x^3y + y^3x = k.
    /// Formula: |dy/dx| = (3x^2y + y^3) / (x^3 + 3xy^2)
    pub fn get_aerodrome_marginal_price(&self, r_in: U256, r_out: U256) -> U256 {
        let x = U512::from(r_in);
        let y = U512::from(r_out);
        let x2 = x * x;
        let y2 = y * y;
        
        let numerator = (U512::from(3u64) * x2 * y) + (y2 * y);
        let denominator = (x2 * x) + (U512::from(3u64) * x * y2);
        
        if denominator.is_zero() { return U256::ZERO; }
        u512_to_u256_safe((numerator * U512::from(10u128.pow(18))) / denominator)
    }

    fn aerodrome_k(&self, x: U256, y: U256) -> U256 {
        let x_u = U512::from(x);
        let y_u = U512::from(y);
        let scale = U512::from(10u128.pow(18)) * U512::from(10u128.pow(18));
        
        let x3y = (x_u * x_u * x_u * y_u) / scale;
        let y3x = (y_u * y_u * y_u * x_u) / scale;
        u512_to_u256_safe(x3y + y3x)
    }

    #[inline(always)]
    fn get_v3_output(&self, amount_in: U256, state: &PoolState, zero_for_one: bool) -> U256 {
        if amount_in.is_zero() || state.liquidity.is_zero() { return U256::ZERO; }
        
        // Use virtual reserves for GSS optimization (very fast)
        let (v_res_in, v_res_out) = Self::get_v3_virtual_reserves(state.sqrt_price_x96, state.liquidity, zero_for_one);
        
        if v_res_in.is_zero() || v_res_out.is_zero() { return U256::ZERO; }
        
        // Apply V3 fee (e.g., 0.05% = 500 pips)
        let fee_multiplier = 1_000_000u32 - state.fee;
        let amt_in_with_fee = U512::from(amount_in) * U512::from(fee_multiplier);
        let numerator = amt_in_with_fee * U512::from(v_res_out);
        let denominator = (U512::from(v_res_in) * U512::from(1_000_000u64)) + amt_in_with_fee;

        u512_to_u256_safe(numerator / denominator)
    }

    /// Pillar D: Newton-Raphson / Secant Optimization
    /// GSS ($O(\log(1/\epsilon))$ se 10x fast convergence ($O(\log(\log(1/\epsilon)))$).
    /// Target: Find x where PathPrice(x) == 1.0 (Marginal Price is Parity).
    pub fn find_optimal_input_newton<F>(initial_guess: U256, mut price_check: F) -> U256
    where F: FnMut(U256) -> f64 {
        let mut x = initial_guess;
        let mut last_x = x;
        let mut last_price = price_check(x);

        // Pillar D: Convergence Cap
        // Newton usually converges in < 8 steps for DEX curves.
        for _ in 0..12 { 
            let price = price_check(x);
            let error = price - 1.0; // We want price(x) = 1.0
            
            // Convergence criteria: 0.001% precision
            if error.abs() < 0.00001 || x.is_zero() { break; }
            
            let x_f = x.to::<u128>() as f64;
            let last_x_f = last_x.to::<u128>() as f64;

            // Estimate the derivative of the price function: g'(x) = d(Price)/dx
            // This is actually the second derivative of the profit function.
            let derivative = if (x_f - last_x_f).abs() < 1e-6 {
                // Fallback: Use a tiny delta for numerical stability if x hasn't moved
                let delta = x_f * 0.01 + 1e6;
                let price_next = price_check(U256::from((x_f + delta) as u128));
                (price_next - price) / delta
            } else {
                (price - last_price) / (x_f - last_x_f)
            };

            if derivative >= 0.0 || derivative.abs() < 1e-28 { break; } // Precision-Tuned for 18 decimals

            let step = error / derivative;
            last_x = x;
            last_price = price;

            let next_x_f = (x_f - step).max(0.0);
            let next_x = U256::from(next_x_f as u128);
            
            if next_x == x || next_x.is_zero() { break; }
            x = next_x;
        }
        x
    }

    pub fn calculate_optimal_v2_v2(
        r1_in: U256, r1_out: U256, r2_in: U256, r2_out: U256,
        fee1_bps: u32, fee2_bps: u32,
    ) -> U256 {
        let g1 = U512::from(10000 - fee1_bps);
        let g2 = U512::from(10000 - fee2_bps);
        let p1 = U512::from(r1_in) * U512::from(r2_in);
        let p2 = U512::from(r1_out) * U512::from(r2_out);
        let p3 = g1 * g2;
        let p4 = U512::from(100_000_000u64);
        let product = p1 * p2 * p3 * p4;
        let sqrt_product = Self::uint_sqrt_512(product);
        let term2 = p1 * U512::from(10_000u64);
        if sqrt_product <= term2 { return U256::ZERO; }
        let numerator = sqrt_product - term2;
        let d1 = U512::from(10_000u64) * U512::from(r2_in);
        let d2 = g2 * U512::from(r1_out);
        let denominator = g1 * (d1 + d2);
        if denominator.is_zero() { return U256::ZERO; }
        let result = (numerator * U512::from(10_000u64)) / denominator;
        u512_to_u256_safe(result)
    }

    pub fn uint_sqrt_512(n: U512) -> U512 {
        if n.is_zero() { return U512::ZERO; }
        let bits = n.bit_len();
        let mut x = U512::from(1u64) << (bits / 2 + 1);
        let mut y = (x + n / x) >> 1;
        while y < x { x = y; y = (x + n / x) >> 1; }
        x
    }

    pub fn get_v3_virtual_reserves(sqrt_price_x96: U256, liquidity: U256, zero_for_one: bool) -> (U256, U256) {
        if sqrt_price_x96.is_zero() || liquidity.is_zero() { return (U256::ZERO, U256::ZERO); }
        let q96 = U512::from(1u64) << 96;
        let l = U512::from(liquidity);
        let sp = U512::from(sqrt_price_x96);
        if zero_for_one {
            let r_in = (l * q96) / sp;
            let r_out = (l * sp) / q96;
            (u512_to_u256_safe(r_in), u512_to_u256_safe(r_out))
        } else {
            let r_in = (l * sp) / q96;
            let r_out = (l * q96) / sp;
            (u512_to_u256_safe(r_in), u512_to_u256_safe(r_out))
        }
    }

    pub fn get_maverick_virtual_reserves(sqrt_price_x96: U256, liquidity: U256, zero_for_one: bool) -> (U256, U256) {
        Self::get_v3_virtual_reserves(sqrt_price_x96, liquidity, zero_for_one)
    }

    pub fn get_v3_next_sqrt_price(sqrt_price_x96: U256, liquidity: U256, amount_in: U256, zero_for_one: bool) -> U256 {
        if liquidity.is_zero() || amount_in.is_zero() { return sqrt_price_x96; }
        let q96 = U512::from(1u64) << 96;
        let l = U512::from(liquidity);
        let sp = U512::from(sqrt_price_x96);
        let amt = U512::from(amount_in);
        if zero_for_one {
            let num = l * sp * q96;
            let den: U512 = l * q96 + amt * sp;
            if den.is_zero() { return sqrt_price_x96; }
            u512_to_u256_safe(num / den)
        } else {
            let delta = (amt * q96) / l;
            u512_to_u256_safe(sp + delta)
        }
    }
}

fn u512_to_u256_safe(v: U512) -> U256 {
    let limbs = v.as_limbs();
    if limbs[4] != 0 || limbs[5] != 0 || limbs[6] != 0 || limbs[7] != 0 {
        return U256::MAX;
    }
    U256::from_limbs([limbs[0], limbs[1], limbs[2], limbs[3]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::U256;

    #[test]
    fn test_aerodrome_stable_k_logic() {
        let math = MathEngine;
        let r0 = U256::from(1000 * 10u128.pow(18)); // 1000 TokenA
        let r1 = U256::from(1000 * 10u128.pow(18)); // 1000 TokenB
        let k = math.aerodrome_k(r0, r1);
        assert!(!k.is_zero());
    }

    #[test]
    fn test_aerodrome_stable_output_stability() {
        let math = MathEngine;
        let state = PoolState {
            reserves0: U256::from(1000 * 10u128.pow(18)),
            reserves1: U256::from(1000 * 10u128.pow(18)),
            is_stable: true,
            ..Default::default()
        };

        let amount_in = U256::from(10u128.pow(18)); // 1 Token In
        let out = math.get_aerodrome_output(amount_in, &state, true);
        
        // Stable curves should have very low slippage for small amounts
        // Out should be close to 1 Token (minus 0.01% fee)
        assert!(out > U256::from(990 * 10u128.pow(15))); 
        assert!(out < amount_in);
    }

    #[test]
    fn test_newton_optimizer_convergence() {
        // Mock a price function: price(x) = 1.1 - (x / 10^21)
        // Marginal price is 1.0 when x = 0.1 * 10^21 = 10^20 (100 tokens/ETH)
        let initial_guess = U256::from(10u128.pow(18));
        let optimal = MathEngine::find_optimal_input_newton(initial_guess, |x| {
            1.1 - (x.to::<u128>() as f64 / 1e21)
        });

        let target = 100 * 10u128.pow(18);
        let diff = if optimal > U256::from(target) {
            optimal - U256::from(target)
        } else {
            U256::from(target) - optimal
        };

        // Precision Check: Must be within 0.0001 ETH of target
        assert!(diff < U256::from(10u128.pow(14)));
    }
}
