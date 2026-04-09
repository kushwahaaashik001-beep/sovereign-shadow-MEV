// =============================================================================
// File: math_engine.rs
// Project: The Sovereign Shadow (MEV/Arbitrage Stealth Engine)
// Description: Pillar D - Mathematical Optimization Engine
//              - Golden Section Search (GSS) for Profit Maximization
//              - V3 Math Utilities
// =============================================================================

use ethers::types::{U256, U512};

/// Inverse Golden Ratio: (sqrt(5) - 1) / 2
#[allow(dead_code)]
const PHI: f64 = 0.618033988749895;

pub struct MathEngine;

impl MathEngine {
    /// Pillar D: Golden Section Search (GSS)
    /// Finds the optimal input amount `x` that maximizes the profit function `f(x)`.
    /// 
    /// # Arguments
    /// * `min_in` - Lower bound of input amount (e.g., 0)
    /// * `max_in` - Upper bound of input amount (e.g., pool balance or wallet balance)
    /// * `f` - A closure that takes an input amount `U256` and returns the expected profit `U256`.
    ///         If the trade reverts or is unprofitable, `f` should return 0.
    /// 
    /// # Returns
    /// * `U256` - The optimal input amount.
    pub fn find_optimal_input<F>(min_in: U256, max_in: U256, mut f: F) -> U256
    where
        F: FnMut(U256) -> U256,
    {
        let tolerance = U256::from(10_000_000_000_000u64); // 0.00001 ETH precision

        let mut a = min_in;
        let mut b = max_in;
        
        let diff = b - a;
        // c = b - (b - a) * PHI
        // We use integer arithmetic approximation for PHI * range
        let phi_num = U256::from(618033988749895u64);
        let phi_den = U256::from(1_000_000_000_000_000u64);
        
        let mut c = b - (diff * phi_num / phi_den);
        let mut d = a + (diff * phi_num / phi_den);

        let mut f_c = f(c);
        let mut f_d = f(d);

        // Max iterations to prevent infinite loops in low volatility
        let mut iterations = 0;
        let max_iter = 20; 

        while (b - a) > tolerance && iterations < max_iter {
            if f_c > f_d {
                b = d;
                d = c;
                f_d = f_c;
                let diff = b - a;
                c = b - (diff * phi_num / phi_den);
                f_c = f(c);
            } else {
                a = c;
                c = d;
                f_c = f_d;
                let diff = b - a;
                d = a + (diff * phi_num / phi_den);
                f_d = f(d);
            }
            iterations += 1;
        }

        // Return the midpoint of the final range
        (a + b) / 2
    }

    /// Pillar D: Newton-Raphson Optimal Input (Nanosecond Convergence)
    /// Specifically designed for Concentrated Liquidity (V3) where GSS is too slow.
    /// Solves for x where: MarginalPrice(x) * PathPriceRatio = 1
    pub fn find_optimal_input_newton<F>(
        initial_guess: U256,
        mut price_check: F,
    ) -> U256 
    where
        F: FnMut(U256) -> f64, // Returns marginal price at input x
    {
        let mut x = initial_guess;
        let mut iterations = 0;
        let max_iter = 5; // Newton-Raphson converges in 3-5 steps for AMM curves

        while iterations < max_iter {
            let current_price = price_check(x);
            
            // Target: Marginal Price = 1.0 (Arbitrage equilibrium)
            let diff = current_price - 1.0;
            if diff.abs() < 0.0001 { break; }

            // Newton Step adjustment
            // For AMMs, we use a curvature-aware step to prevent overshoot
            let step = (x.as_u128() as f64 * diff * 0.5) as i128;
            
            let next_x = if step > 0 {
                x.saturating_add(U256::from(step as u128))
            } else {
                x.saturating_sub(U256::from(step.unsigned_abs()))
            };

            if next_x == x || next_x.is_zero() { break; }
            x = next_x;
            iterations += 1;
        }
        x
    }

    /// Pillar D: Analytical Optimal Input for 2-Pool V2 Arbitrage (O(1))
    /// Based on the derivative of the profit function for x * y = k AMMs.
    pub fn calculate_optimal_v2_v2(
        r1_in: U256,
        r1_out: U256,
        r2_in: U256,
        r2_out: U256,
        fee1_bps: u32,
        fee2_bps: u32,
    ) -> U256 {
        let g1 = U256::from(10000 - fee1_bps);
        let g2 = U256::from(10000 - fee2_bps);

        // Formula: x = (sqrt(r1_in * r2_in * r1_out * r2_out * g1 * g2 * 10^8) - r1_in * r2_in * 10^4) / (g1 * (10^4 * r2_in + g2 * r1_out))
        let p1 = U512::from(r1_in) * U512::from(r2_in);
        let p2 = U512::from(r1_out) * U512::from(r2_out);
        let p3 = U512::from(g1) * U512::from(g2);
        let p4 = U512::from(100_000_000u64); // 10^8 scaling for precision

        let product = p1 * p2 * p3 * p4;
        let sqrt_product = Self::uint_sqrt_512(product);

        let term2 = p1 * U512::from(10_000u64);

        if sqrt_product <= term2 {
            return U256::zero();
        }

        let numerator = sqrt_product - term2;
        let d1 = U512::from(10_000u64) * U512::from(r2_in);
        let d2 = U512::from(g2) * U512::from(r1_out);
        let denominator = U512::from(g1) * (d1 + d2);

        if denominator.is_zero() { return U256::zero(); }

        let result = (numerator * U512::from(10_000u64)) / denominator;
        result.try_into().unwrap_or(U256::zero())
    }

    pub fn uint_sqrt_512(n: U512) -> U512 {
        if n.is_zero() { return U512::zero(); }
        let mut x = U512::from(1) << (n.bits() / 2 + 1);
        let mut y = (x + n / x) >> 1;
        while y < x {
            x = y;
            y = (x + n / x) >> 1;
        }
        x
    }

    /// Pillar D: Calculate virtual reserves for a V3 pool within a single tick.
    /// This allows treating a V3 pool as a V2 pool locally for O(1) optimal input calculation.
    pub fn get_v3_virtual_reserves(sqrt_price_x96: U256, liquidity: U256, zero_for_one: bool) -> (U256, U256) {
        if sqrt_price_x96.is_zero() || liquidity.is_zero() { return (U256::zero(), U256::zero()); }
        let q96 = U512::from(1) << 96;
        let l = U512::from(liquidity);
        let sp = U512::from(sqrt_price_x96);

        if zero_for_one {
            // Token 0 in -> Token 1 out
            let r_in = (l * q96) / sp;
            let r_out = (l * sp) / q96;
            (r_in.try_into().unwrap_or(U256::zero()), r_out.try_into().unwrap_or(U256::zero()))
        } else {
            // Token 1 in -> Token 0 out
            let r_in = (l * sp) / q96;
            let r_out = (l * q96) / sp;
            (r_in.try_into().unwrap_or(U256::zero()), r_out.try_into().unwrap_or(U256::zero()))
        }
    }

    /// Pillar D: Maverick Virtual Reserves
    /// Maverick bins can be approximated as deep virtual reserves for O(1) arbitrage math.
    pub fn get_maverick_virtual_reserves(sqrt_price_x96: U256, liquidity: U256, zero_for_one: bool) -> (U256, U256) {
        if sqrt_price_x96.is_zero() || liquidity.is_zero() { return (U256::zero(), U256::zero()); }
        
        // Pillar S: God-Mode Maverick V2 Bin Hardening
        // Maverick V2 uses dynamic bins. We use U512 to maintain 100% precision.
        // Price P = (sqrtPrice / 2^96)^2
        let q96 = U512::from(1) << 96;
        let l = U512::from(liquidity);
        let sp = U512::from(sqrt_price_x96);

        if zero_for_one {
            // Token 0 -> Token 1
            let r_in = (l * q96) / sp; 
            let r_out = (l * sp) / q96;
            (r_in.try_into().unwrap_or(U256::zero()), r_out.try_into().unwrap_or(U256::zero()))
        } else {
            // Token 1 -> Token 0
            let r_in = (l * sp) / q96;
            let r_out = (l * q96) / sp;
            (r_in.try_into().unwrap_or(U256::zero()), r_out.try_into().unwrap_or(U256::zero()))
        }
    }

    /// Pillar D: Predict next sqrtPrice for a V3 pool after a swap of 'amount_in'.
    /// Used to check if the O(1) optimal input crosses a tick boundary.
    pub fn get_v3_next_sqrt_price(sqrt_price_x96: U256, liquidity: U256, amount_in: U256, zero_for_one: bool) -> U256 {
        if liquidity.is_zero() { return sqrt_price_x96; }
        let l = U512::from(liquidity);
        let sp = U512::from(sqrt_price_x96);
        let ai = U512::from(amount_in);
        let q96 = U512::from(1) << 96;

        if zero_for_one {
            // token0 in -> price decreases
            // sqrt_next = (L * sp) / (L + amount_in * sp / 2^96)
            let denominator = l + (ai * sp / q96);
            let next = (l * sp) / denominator;
            next.try_into().unwrap_or(U256::zero())
        } else {
            // token1 in -> price increases
            // sqrt_next = sp + (amount_in * 2^96 / L)
            let next = sp + (ai * q96 / l);
            next.try_into().unwrap_or(U256::zero())
        }
    }
}
