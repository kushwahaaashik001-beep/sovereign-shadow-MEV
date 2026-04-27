use alloy_primitives::{I256, U256, Uint};
type U512 = Uint<512, 8>;
use once_cell::sync::Lazy;
use rustc_hash::FxHashMap;
use std::cmp::Ordering;
use std::sync::Arc;

pub const MIN_TICK: i32 = -887272;
pub const MAX_TICK: i32 = 887272;
pub const GAS_PER_TICK_CROSS: u64 = 5_000;

#[inline] pub fn q96()  -> U256 { U256::from(1u128) << 96 }
#[inline] pub fn q128() -> U256 { U256::from(1u128) << 128 }
#[inline] pub fn min_sqrt_ratio() -> U256 { U256::from(4295128739u64) }
#[inline] pub fn max_sqrt_ratio() -> U256 {
    U256::from_limbs([0x5d951d5263988d25, 0xefd1fc6a50648849, 0xfffd8963, 0])
}

static SQRT_RATIO_LUT: Lazy<Vec<U256>> = Lazy::new(|| {
    (-110000i32..=110000i32).map(get_sqrt_ratio_at_tick_uncached).collect()
});

#[inline(always)]
pub fn fee_to_tick_spacing(fee: u32) -> i32 {
    match fee { 100 => 1, 500 => 10, 3000 => 60, 10000 => 200, _ => 1 }
}

#[inline(always)]
pub fn mul_div(a: U256, b: U256, denominator: U256) -> Option<U256> {
    if denominator.is_zero() { return None; }
    let a512 = U512::from(a);
    let b512 = U512::from(b);
    let d512 = U512::from(denominator);
    let result = (a512 * b512) / d512;
    Some(u512_to_u256_safe(result))
}

#[inline(always)]
pub fn mul_div_round_up(a: U256, b: U256, denominator: U256) -> Option<U256> {
    if denominator.is_zero() { return None; }
    let a512 = U512::from(a);
    let b512 = U512::from(b);
    let d512 = U512::from(denominator);
    let product = a512 * b512;
    let result = (product + d512 - U512::from(1u64)) / d512;
    Some(u512_to_u256_safe(result))
}

#[inline(always)]
pub fn mul_div_round_down(a: U256, b: U256, denominator: U256) -> Option<U256> {
    mul_div(a, b, denominator)
}

fn u512_to_u256_safe(v: U512) -> U256 {
    let limbs = v.as_limbs();
    if limbs[4] != 0 || limbs[5] != 0 || limbs[6] != 0 || limbs[7] != 0 {
        return U256::MAX;
    }
    U256::from_limbs([limbs[0], limbs[1], limbs[2], limbs[3]])
}

#[inline(always)]
pub fn mul_shift_128(a: U256, b: u128) -> U256 {
    let a512 = U512::from(a);
    let b512 = U512::from(b);
    let result = (a512 * b512) >> 128;
    u512_to_u256_safe(result)
}

#[inline(always)]
pub fn get_sqrt_ratio_at_tick(tick: i32) -> U256 {
    if tick >= -110000 && tick <= 110000 {
        return SQRT_RATIO_LUT[(tick + 110000) as usize];
    }
    get_sqrt_ratio_at_tick_uncached(tick)
}

fn get_sqrt_ratio_at_tick_uncached(tick: i32) -> U256 {
    let abs_tick = tick.unsigned_abs();
    if abs_tick > MAX_TICK as u32 { return U256::ZERO; }

    let mut ratio = if abs_tick & 0x1 != 0 {
        U256::from_limbs([0xaa2d162d1a594001, 0xfffcb933bd6fad37, 0, 0])
    } else {
        q128()
    };

    if abs_tick & 0x2 != 0 { ratio = mul_shift_128(ratio, 0xfff97272373d413259a46990580e213a); }
    if abs_tick & 0x4 != 0 { ratio = mul_shift_128(ratio, 0xfff2e50f5f656932ef12357cf3c7fdcc); }
    if abs_tick & 0x8 != 0 { ratio = mul_shift_128(ratio, 0xffe5caca7e10e4e61c3624eaa0941cd0); }
    if abs_tick & 0x10 != 0 { ratio = mul_shift_128(ratio, 0xffcb9843d60f6159c9db58835c926644); }
    if abs_tick & 0x20 != 0 { ratio = mul_shift_128(ratio, 0xff973b41fa98c081472e6896dfb254c0); }
    if abs_tick & 0x40 != 0 { ratio = mul_shift_128(ratio, 0xff2ea16466c96a3843ec78b326b52861); }
    if abs_tick & 0x80 != 0 { ratio = mul_shift_128(ratio, 0xfe5dee046a99a2a811c461f1969c3053); }
    if abs_tick & 0x100 != 0 { ratio = mul_shift_128(ratio, 0xfcbe86c7900a88aedcffc83b479aa3a4); }
    if abs_tick & 0x200 != 0 { ratio = mul_shift_128(ratio, 0xf987a7253ac413176f2b074cf7815e54); }
    if abs_tick & 0x400 != 0 { ratio = mul_shift_128(ratio, 0xf3392b0822b70005940c7a398e4b70f3); }
    if abs_tick & 0x800 != 0 { ratio = mul_shift_128(ratio, 0xe7159475a2c29b7443b29c7fa6e889d9); }
    if abs_tick & 0x1000 != 0 { ratio = mul_shift_128(ratio, 0xd097f3bdfd2022b8845ad8f792aa5825); }
    if abs_tick & 0x2000 != 0 { ratio = mul_shift_128(ratio, 0xa9f746462d870fdf8a65dc1f90e061e5); }
    if abs_tick & 0x4000 != 0 { ratio = mul_shift_128(ratio, 0x70d869a156d2a1b890bb3df62baf32f7); }
    if abs_tick & 0x8000 != 0 { ratio = mul_shift_128(ratio, 0x31be135f97d08fd981231505542fcfa6); }
    if abs_tick & 0x10000 != 0 { ratio = mul_shift_128(ratio, 0x9aa508b5b7a84e1c677de54f3e99bc9); }
    if abs_tick & 0x20000 != 0 { ratio = mul_shift_128(ratio, 0x5d6af8dedb81196699c329225ee604); }
    if abs_tick & 0x40000 != 0 { ratio = mul_shift_128(ratio, 0x2216e584f5fa1ea926041bedfe98); }
    if abs_tick & 0x80000 != 0 { ratio = mul_shift_128(ratio, 0x48a170391f7dc42444e8fa2); }

    if tick > 0 { ratio = U256::MAX / ratio; }

    // Pillar V3: Avoid .to::<u128>() panic for q128 (2^128)
    let low_bits_exist = !(ratio & U256::from(0xFFFFFFFFu32)).is_zero();
    let shifted = ratio >> 32;
    shifted + U256::from(low_bits_exist as u64)
}

#[inline(always)]
pub fn get_tick_at_sqrt_ratio(sqrt_price_x96: U256) -> i32 {
    let mut low = MIN_TICK;
    let mut high = MAX_TICK;
    while low <= high {
        let mid = (low + high) / 2;
        let mid_price = get_sqrt_ratio_at_tick(mid);
        match mid_price.cmp(&sqrt_price_x96) {
            Ordering::Less => low = mid + 1,
            Ordering::Greater => high = mid - 1,
            Ordering::Equal => return mid,
        }
    }
    high
}

pub fn get_amount_0_for_liquidity(lower: U256, upper: U256, liquidity: U256) -> U256 {
    let (lo, hi) = if lower <= upper { (lower, upper) } else { (upper, lower) };
    let delta = hi - lo;
    let q = q96();
    let num = mul_div(liquidity, delta, U256::from(1u64)).unwrap_or(U256::ZERO);
    let den = mul_div(hi, lo, q).unwrap_or(U256::MAX);
    if den.is_zero() { return U256::ZERO; }
    mul_div(num, U256::from(1u64), den).unwrap_or(U256::ZERO)
}

pub fn get_amount_1_for_liquidity(lower: U256, upper: U256, liquidity: U256) -> U256 {
    let delta = upper.saturating_sub(lower);
    mul_div(liquidity, delta, q96()).unwrap_or(U256::ZERO)
}

pub fn get_liquidity_for_amount_0(lower: U256, upper: U256, amount0: U256) -> U256 {
    let delta = upper.saturating_sub(lower);
    if delta.is_zero() { return U256::ZERO; }
    let den = mul_div(upper, lower, q96()).unwrap_or(U256::ZERO);
    mul_div(amount0, den, delta).unwrap_or(U256::ZERO)
}

pub fn get_liquidity_for_amount_1(lower: U256, upper: U256, amount1: U256) -> U256 {
    let delta = upper.saturating_sub(lower);
    if delta.is_zero() { return U256::ZERO; }
    amount1 * q96() / delta
}

#[derive(Debug, Clone)]
pub struct SwapStepResult {
    pub sqrt_price_next_x96: U256,
    pub amount_in: U256,
    pub amount_out: U256,
    pub fee_amount: U256,
}

pub fn compute_swap_step(
    sqrt_price_current: U256,
    sqrt_price_target: U256,
    liquidity: U256,
    amount_remaining: I256,
    fee_pips: u32,
    zero_for_one: bool,
) -> (SwapStepResult, I256) {
    let mut result = SwapStepResult {
        sqrt_price_next_x96: U256::ZERO,
        amount_in: U256::ZERO,
        amount_out: U256::ZERO,
        fee_amount: U256::ZERO,
    };

    let exact_input = amount_remaining > I256::ZERO;

    if exact_input {
        let amount_remaining_less_fee = mul_div(
            U256::from_be_bytes(amount_remaining.abs().to_be_bytes::<32>()),
            U256::from(1_000_000 - fee_pips),
            U256::from(1_000_000),
        ).unwrap_or(U256::ZERO);

        result.amount_in = if zero_for_one {
            get_amount0_delta(sqrt_price_target, sqrt_price_current, liquidity, true)
        } else {
            get_amount1_delta(sqrt_price_current, sqrt_price_target, liquidity, true)
        };

        if amount_remaining_less_fee >= result.amount_in {
            result.sqrt_price_next_x96 = sqrt_price_target;
        } else {
            result.sqrt_price_next_x96 = get_next_sqrt_price_from_input(
                sqrt_price_current,
                liquidity,
                amount_remaining_less_fee,
                zero_for_one,
            );
        }
    } else {
        result.amount_out = if zero_for_one {
            get_amount1_delta(sqrt_price_target, sqrt_price_current, liquidity, false)
        } else {
            get_amount0_delta(sqrt_price_current, sqrt_price_target, liquidity, false)
        };

        let amount_out_abs = U256::from_be_bytes(amount_remaining.abs().to_be_bytes::<32>());
        if amount_out_abs >= result.amount_out {
            result.sqrt_price_next_x96 = sqrt_price_target;
        } else {
            result.sqrt_price_next_x96 = get_next_sqrt_price_from_output(
                sqrt_price_current,
                liquidity,
                amount_out_abs,
                zero_for_one,
            );
        }
    }

    let max = result.sqrt_price_next_x96 == sqrt_price_target;

    if zero_for_one {
        if !max || !exact_input {
            result.amount_in = get_amount0_delta(result.sqrt_price_next_x96, sqrt_price_current, liquidity, true);
        }
        if !max || exact_input {
            result.amount_out = get_amount1_delta(result.sqrt_price_next_x96, sqrt_price_current, liquidity, false);
        }
    } else {
        if !max || !exact_input {
            result.amount_in = get_amount1_delta(sqrt_price_current, result.sqrt_price_next_x96, liquidity, true);
        }
        if !max || exact_input {
            result.amount_out = get_amount0_delta(sqrt_price_current, result.sqrt_price_next_x96, liquidity, false);
        }
    }

    if exact_input && result.sqrt_price_next_x96 != sqrt_price_target {
        result.fee_amount = U256::from_be_bytes(amount_remaining.abs().to_be_bytes::<32>()) - result.amount_in;
    } else {
        result.fee_amount = mul_div_round_up(
            result.amount_in,
            U256::from(fee_pips),
            U256::from(1_000_000 - fee_pips),
        ).unwrap_or(U256::ZERO);
    }

    let rem = if exact_input {
        amount_remaining - I256::from_be_bytes((result.amount_in + result.fee_amount).to_be_bytes::<32>())
    } else {
        amount_remaining + I256::from_be_bytes(result.amount_out.to_be_bytes::<32>())
    };

    (result, rem)
}

pub fn simulate_swap_with_limit(
    mut sqrt_price_x96: U256,
    mut tick: i32,
    mut liquidity: U256,
    ticks: &Arc<FxHashMap<i32, (i128, u128)>>,
    tick_bitmap: &Arc<FxHashMap<i16, U256>>,
    amount_in: U256,
    zero_for_one: bool,
    fee: u32,
    sqrt_price_limit_x96: Option<U256>,
    tick_spacing: i32,
) -> (U256, U256, i32, U256) {
    let mut amount_remaining = I256::from_be_bytes(amount_in.to_be_bytes::<32>());
    let mut amount_calculated = U256::ZERO;
    let sqrt_price_limit = sqrt_price_limit_x96.unwrap_or(if zero_for_one { min_sqrt_ratio() + U256::from(1) } else { max_sqrt_ratio() - U256::from(1) });

    while amount_remaining != I256::ZERO && sqrt_price_x96 != sqrt_price_limit {
        let (next_tick, _initialized) = get_next_initialized_tick_precise(tick, tick_spacing, zero_for_one, tick_bitmap);
        let next_sqrt_price = get_sqrt_ratio_at_tick(next_tick);

        let target_price = if zero_for_one {
            if next_sqrt_price < sqrt_price_limit { sqrt_price_limit } else { next_sqrt_price }
        } else {
            if next_sqrt_price > sqrt_price_limit { sqrt_price_limit } else { next_sqrt_price }
        };

        let (step, rem) = compute_swap_step(sqrt_price_x96, target_price, liquidity, amount_remaining, fee, zero_for_one);
        
        amount_remaining = rem;
        amount_calculated += step.amount_out;
        sqrt_price_x96 = step.sqrt_price_next_x96;

        if sqrt_price_x96 == next_sqrt_price {
            if let Some((liquidity_net, _)) = ticks.get(&next_tick) {
                if zero_for_one {
                    liquidity = liquidity.saturating_sub(U256::from(liquidity_net.unsigned_abs()));
                } else {
                    liquidity = liquidity.saturating_add(U256::from(liquidity_net.unsigned_abs()));
                }
            }
            tick = if zero_for_one { next_tick - 1 } else { next_tick };
        } else {
            tick = get_tick_at_sqrt_ratio(sqrt_price_x96);
        }
    }

    (amount_calculated, sqrt_price_x96, tick, liquidity)
}

#[inline(always)]
pub fn get_amount0_delta(
    sqrt_ratio_a_x96: U256,
    sqrt_ratio_b_x96: U256,
    liquidity: U256,
    round_up: bool,
) -> U256 {
    let (sqrt_ratio_a, sqrt_ratio_b) = if sqrt_ratio_a_x96 > sqrt_ratio_b_x96 {
        (sqrt_ratio_b_x96, sqrt_ratio_a_x96)
    } else {
        (sqrt_ratio_a_x96, sqrt_ratio_b_x96)
    };

    let numerator1 = liquidity << 96;
    let numerator2 = sqrt_ratio_b - sqrt_ratio_a;

    if round_up {
        mul_div_round_up(mul_div_round_up(numerator1, numerator2, sqrt_ratio_b).unwrap_or(U256::ZERO), U256::from(1), sqrt_ratio_a).unwrap_or(U256::ZERO)
    } else {
        mul_div(mul_div(numerator1, numerator2, sqrt_ratio_b).unwrap_or(U256::ZERO), U256::from(1), sqrt_ratio_a).unwrap_or(U256::ZERO)
    }
}

#[inline(always)]
pub fn get_amount1_delta(
    sqrt_ratio_a_x96: U256,
    sqrt_ratio_b_x96: U256,
    liquidity: U256,
    round_up: bool,
) -> U256 {
    let (sqrt_ratio_a, sqrt_ratio_b) = if sqrt_ratio_a_x96 > sqrt_ratio_b_x96 {
        (sqrt_ratio_b_x96, sqrt_ratio_a_x96)
    } else {
        (sqrt_ratio_a_x96, sqrt_ratio_b_x96)
    };

    if round_up {
        mul_div_round_up(liquidity, sqrt_ratio_b - sqrt_ratio_a, q96()).unwrap_or(U256::ZERO)
    } else {
        mul_div(liquidity, sqrt_ratio_b - sqrt_ratio_a, q96()).unwrap_or(U256::ZERO)
    }
}

pub fn get_next_sqrt_price_from_input(
    sqrt_price_x96: U256,
    liquidity: U256,
    amount_in: U256,
    zero_for_one: bool,
) -> U256 {
    if zero_for_one {
        // get_next_sqrt_price_from_amount0_rounding_up
        let numerator = liquidity << 96;
        let product = amount_in * sqrt_price_x96;
        if product / amount_in == sqrt_price_x96 {
            let denominator = numerator + product;
            if denominator >= numerator {
                return mul_div_round_up(numerator, sqrt_price_x96, denominator).unwrap_or(U256::ZERO);
            }
        }
        mul_div_round_up(numerator, U256::from(1), (numerator / sqrt_price_x96) + amount_in).unwrap_or(U256::ZERO)
    } else {
        // get_next_sqrt_price_from_amount1_rounding_down
        let quotient = (amount_in << 96) / liquidity;
        sqrt_price_x96 + quotient
    }
}

pub fn get_next_sqrt_price_from_output(
    sqrt_price_x96: U256,
    liquidity: U256,
    amount_out: U256,
    zero_for_one: bool,
) -> U256 {
    if zero_for_one {
        // get_next_sqrt_price_from_amount1_rounding_down
        let quotient = (amount_out << 96) / liquidity;
        sqrt_price_x96 - quotient
    } else {
        // get_next_sqrt_price_from_amount0_rounding_up
        let numerator = liquidity << 96;
        let product = amount_out * sqrt_price_x96;
        let denominator = numerator - product;
        mul_div_round_up(numerator, sqrt_price_x96, denominator).unwrap_or(U256::ZERO)
    }
}

pub fn get_next_initialized_tick_precise(
    tick: i32,
    tick_spacing: i32,
    lte: bool,
    tick_bitmap: &FxHashMap<i16, U256>,
) -> (i32, bool) {
    let mut compressed = tick / tick_spacing;
    if tick < 0 && tick % tick_spacing != 0 { compressed -= 1; }

    if lte {
        let (word_pos, bit_pos) = tick_bitmap_position(compressed);
        let mask = (U256::from(1) << bit_pos) - U256::from(1) + (U256::from(1) << bit_pos);
        let masked = tick_bitmap.get(&word_pos).cloned().unwrap_or_default() & mask;

        let initialized = !masked.is_zero();
        let next_tick = if initialized {
            (compressed - (bit_pos as i32 - most_significant_bit(masked) as i32)) * tick_spacing
        } else {
            (compressed - bit_pos as i32) * tick_spacing
        };
        (next_tick, initialized)
    } else {
        let (word_pos, bit_pos) = tick_bitmap_position(compressed + 1);
        let mask = !((U256::from(1) << bit_pos) - U256::from(1));
        let masked = tick_bitmap.get(&word_pos).cloned().unwrap_or_default() & mask;

        let initialized = !masked.is_zero();
        let next_tick = if initialized {
            (compressed + 1 + (least_significant_bit(masked) as i32 - bit_pos as i32)) * tick_spacing
        } else {
            (compressed + 1 + (255 - bit_pos as i32)) * tick_spacing
        };
        (next_tick, initialized)
    }
}

#[inline(always)]
fn most_significant_bit(x: U256) -> u8 {
    if x.is_zero() { 0 } else { (x.bit_len() - 1) as u8 }
}

#[inline(always)]
fn least_significant_bit(x: U256) -> u8 {
    x.trailing_zeros() as u8
}

#[inline]
pub fn tick_bitmap_position(tick: i32) -> (i16, u8) {
    ((tick >> 8) as i16, (tick as u8))
}

pub struct V3SwapLog {
    pub amount0: I256,
    pub amount1: I256,
    pub sqrt_price: U256,
    pub liquidity: U256,
    pub tick: i32,
}

pub fn decode_v3_swap_log(log: &alloy::rpc::types::Log) -> Option<V3SwapLog> {
    let data = log.data().data.as_ref();
    if data.len() < 160 { return None; }
    
    let amount0 = I256::from_be_bytes::<32>(data[0..32].try_into().ok()?);
    let amount1 = I256::from_be_bytes::<32>(data[32..64].try_into().ok()?);
    let sqrt_price = U256::from_be_slice(&data[64..96]);
    let liquidity = U256::from_be_slice(&data[96..128]);
    let tick = i32::try_from(I256::from_be_bytes::<32>(data[128..160].try_into().ok()?)).ok()?;
    
    Some(V3SwapLog { amount0, amount1, sqrt_price, liquidity, tick })
}

pub fn get_next_initialized_tick(
    tick: i32,
    lte: bool,
    _ticks: &FxHashMap<i32, (i128, u128)>,
    _tick_bitmap: &FxHashMap<i16, U256>,
) -> i32 {
    if lte { tick - 1 } else { tick + 1 }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_tick_zero() {
        let price = get_sqrt_ratio_at_tick(0);
        assert_eq!(price, q96());
    }
}
