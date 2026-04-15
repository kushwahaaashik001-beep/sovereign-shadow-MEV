use alloy_primitives::{Address, U256};
use bytes::Bytes;
use phf::phf_map;

use crate::constants;
use crate::models::{DexName, SwapInfo};
use crate::utils::{read_address, read_u256, read_usize};

/// Pillar A: Zero-copy transaction decoder — extracts swap intent from raw calldata.
#[derive(Debug, Clone, Default)]
pub struct UniversalDecoder;

/// Minimal tx representation for decoding.
#[derive(Default, Clone)]
pub struct DecodeTx {
    pub to:    Option<Address>,
    pub value: U256,
    pub input: Bytes,
}

#[derive(Clone, Copy)]
enum SelectorMethod {
    V2,
    Aerodrome,
    V2Eth,
    V3Single,
    UniversalRouter,
    UniswapX,
}

/// Static Lookup Table with Perfect Hash Function (O(1) search)
static SELECTORS: phf::Map<u32, SelectorMethod> = phf_map! {
    0x38ed1739u32 => SelectorMethod::V2,
    0x8803dbeeu32 => SelectorMethod::V2,
    0x18cbafe5u32 => SelectorMethod::V2,
    0x4a25d94au32 => SelectorMethod::V2,
    0xa1251d75u32 => SelectorMethod::Aerodrome,
    0xcdf2de83u32 => SelectorMethod::Aerodrome,
    0x15263a6au32 => SelectorMethod::Aerodrome,
    0x7ff36449u32 => SelectorMethod::V2Eth,
    0xfb3bdb41u32 => SelectorMethod::V2Eth,
    0x414bf389u32 => SelectorMethod::V3Single,
    0x3593564cu32 => SelectorMethod::UniversalRouter,
    0x8ae0693au32 => SelectorMethod::UniswapX,      // execute((bytes,bytes))
    0x5b0d135au32 => SelectorMethod::UniswapX,      // executeBatch((bytes,bytes)[])
};

impl UniversalDecoder {
    pub fn new() -> Self { Self }

    /// Main entry point for Zero-Copy Decoding.
    /// Convert first 4 bytes to u32 for single-cycle CPU comparison.
    pub fn decode(&self, tx: &DecodeTx) -> Vec<SwapInfo> {
        let data = &tx.input;
        if data.len() < 4 { return vec![]; }

        // Fast selector extraction (Zero-Copy)
        let selector = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        let to = tx.to.unwrap_or(Address::ZERO);

        // O(1) Dispatch Logic
        match SELECTORS.get(&selector) {
            Some(SelectorMethod::V2) => self.decode_v2(&data[4..], to).into_iter().collect(),
            Some(SelectorMethod::Aerodrome) => self.decode_aerodrome(data, to, tx.value).into_iter().collect(),
            Some(SelectorMethod::V2Eth) => self.decode_v2_eth(&data[4..], to, tx.value).into_iter().collect(),
            Some(SelectorMethod::V3Single) => self.decode_v3_single(data, to),
            Some(SelectorMethod::UniversalRouter) => self.decode_universal_router(data, to),
            Some(SelectorMethod::UniswapX) => self.decode_uniswapx(data, to),
            None => vec![],
        }
    }

    #[inline(always)]
    fn _is_admin_call(&self, data: &[u8]) -> bool {
        let s = &data[0..4];
        s == constants::SELECTOR_UPGRADE_TO.0
            || s == constants::SELECTOR_UPGRADE_TO_AND_CALL.0
            || s == constants::SELECTOR_SET_FEE.0
    }

    #[inline(always)]
    fn decode_aerodrome(&self, data: &[u8], to: Address, value: U256) -> Option<SwapInfo> {
        if data.len() < 4 { return None; }
        let selector = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        let input = &data[4..];

        let (amount_in, amount_out_min, routes_offset, recipient) = match selector {
            0xcdf2de83 => { // swapExactETHForTokens(uint,Route[],address,uint)
                if input.len() < 96 { return None; }
                (value, read_u256(input, 0)?, read_usize(input, 32)?, read_address(input, 64)?)
            }
            0x15263a6a | 0xa1251d75 => { // swapExactTokensForETH | swapExactTokensForTokens
                if input.len() < 128 { return None; }
                (read_u256(input, 0)?, read_u256(input, 32)?, read_usize(input, 64)?, read_address(input, 96)?)
            }
            _ => return None,
        };

        let routes_len = read_usize(input, routes_offset)?;
        if routes_len == 0 { return None; }
        
        // Pillar Z: Aerodrome Route structure is (address from, address to, bool stable) -> 3 slots (96 bytes)
        let route_start = routes_offset + 32;
        let token_in  = read_address(input, route_start)?;
        
        // Extract final token_out from the last route in the array
        let final_route_idx = route_start + (routes_len - 1) * 96;
        let token_out = read_address(input, final_route_idx + 32)?;

        Some(SwapInfo { 
            dex: DexName::Aerodrome, router: to, token_in, token_out,
            amount_in, amount_out_min, to: recipient, fee: None, permit2_nonce: None 
        })
    }

    #[inline(always)]
    fn decode_v2(&self, data: &[u8], to: Address) -> Option<SwapInfo> {
        if data.len() < 160 { return None; }
        let amount_in      = read_u256(data, 0)?;
        let amount_out_min = read_u256(data, 32)?;
        let path_offset    = read_usize(data, 64)?;
        let path_len       = read_usize(data, path_offset)?;
        if !(2..=10).contains(&path_len) { return None; }
        let path_start = path_offset + 32;
        if data.len() < path_start + path_len * 32 { return None; }
        let token_in  = read_address(data, path_start)?;
        let token_out = read_address(data, path_start + (path_len - 1) * 32)?;
        let recipient = read_address(data, 96)?;
        Some(SwapInfo { dex: DexName::UniswapV2, router: to, token_in, token_out,
                        amount_in, amount_out_min, to: recipient, fee: None, permit2_nonce: None })
    }

    /// Pillar S: UniswapX Intent Decoder.
    /// Extracts input/output tokens from signed Dutch Orders.
    fn decode_uniswapx(&self, data: &[u8], reactor: Address) -> Vec<SwapInfo> {
        if data.len() < 128 { return vec![]; }
        let selector = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        
        let mut intents = Vec::new();

        if selector == 0x8ae0693a { // execute((bytes,bytes))
            if let Some(info) = self.parse_uniswapx_order(data, 4, reactor) {
                intents.push(info);
            }
        } else if selector == 0x5b0d135a { // executeBatch((bytes,bytes)[])
            if let Some(array_offset) = read_usize(data, 4) {
                let array_start = array_offset + 4;
                if let Some(len) = read_usize(data, array_start) {
                    for i in 0..len.min(5) { // Limit to 5 per batch for safety
                        let item_offset = array_start + 32 + (i * 32);
                        if let Some(rel_ptr) = read_usize(data, item_offset) {
                            if let Some(info) = self.parse_uniswapx_order(data, array_start + rel_ptr, reactor) {
                                intents.push(info);
                            }
                        }
                    }
                }
            }
        }
        intents
    }

    fn parse_uniswapx_order(&self, data: &[u8], offset: usize, reactor: Address) -> Option<SwapInfo> {
        // UniswapX structure: offset to bytes order, offset to bytes signature
        let order_ptr = read_usize(data, offset)?;
        let order_abs = offset + order_ptr;
        let order_len = read_usize(data, order_abs)?;
        let order_payload = &data[order_abs + 32 .. (order_abs + 32 + order_len).min(data.len())];

        if order_payload.len() < 160 { return None; }

        // DutchOrder/ExclusiveFillerOrder common layout:
        // Info: [0..128] (Reactor, Swapper, Nonce, Deadline)
        // Exclusivity: [128..192] (ExclFiller, ExclOverride)
        // Input: [192..288] (Token, StartAmt, EndAmt)
        // Outputs: [288..] (Offset to array)
        
        let token_in = read_address(order_payload, 192)?;
        let amount_in = read_u256(order_payload, 204)?; // Use startAmount as base
        
        let outputs_ptr = read_usize(order_payload, 288)?;
        let outputs_abs = outputs_ptr;
        if order_payload.len() < outputs_abs + 64 { return None; }
        
        let outputs_len = read_usize(order_payload, outputs_abs)?;
        if outputs_len == 0 { return None; }
        
        // First output only for arb detection
        let first_output_ptr = outputs_abs + 32;
        let token_out = read_address(order_payload, first_output_ptr)?;
        let amount_out_min = read_u256(order_payload, first_output_ptr + 12)?; // startAmount
        let recipient = read_address(order_payload, first_output_ptr + 76)?;

        Some(SwapInfo {
            dex: DexName::UniswapX,
            router: reactor,
            token_in, token_out, amount_in, amount_out_min,
            to: recipient, fee: None, permit2_nonce: None
        })
    }

    #[inline(always)]
    fn decode_v2_eth(&self, data: &[u8], to: Address, value: U256) -> Option<SwapInfo> {
        if data.len() < 128 { return None; }
        let amount_out_min = read_u256(data, 0)?;
        let path_offset    = read_usize(data, 32)?;
        let recipient      = read_address(data, 64)?;
        let path_len       = read_usize(data, path_offset)?;
        if !(2..=10).contains(&path_len) { return None; }
        let path_start = path_offset + 32;
        let token_in  = read_address(data, path_start)?;
        let token_out = read_address(data, path_start + (path_len - 1) * 32)?;
        Some(SwapInfo { dex: DexName::UniswapV2, router: to, token_in, token_out,
                        amount_in: value, amount_out_min, to: recipient, fee: None, permit2_nonce: None })
    }

    #[inline(always)]
    fn decode_v3_single(&self, data: &[u8], to: Address) -> Vec<SwapInfo> {
        if data.len() < 228 { return vec![]; }
        // Detect if called via Proxy/Multicall (Offset 4 == 32)
        let start = if data.len() >= 36 && read_usize(data, 4) == Some(32) { 36 } else { 4 };
        
        let token_in      = read_address(data, start).unwrap_or_default();
        let token_out     = read_address(data, start + 32).unwrap_or_default();
        let fee           = read_u256(data, start + 64).unwrap_or_default().to::<u32>();
        let recipient     = read_address(data, start + 96).unwrap_or_default();
        let amount_in     = read_u256(data, start + 160).unwrap_or_default();
        let amount_out_min= read_u256(data, start + 192).unwrap_or_default();
        vec![SwapInfo { dex: DexName::UniswapV3, router: to, token_in, token_out,
                        amount_in, amount_out_min, to: recipient, fee: Some(fee), permit2_nonce: None }]
    }

    #[inline(always)]
    fn decode_universal_router(&self, data: &[u8], to: Address) -> Vec<SwapInfo> {
        let data = if data.len() >= 4 { &data[4..] } else { return vec![] };
        if data.len() < 96 { return vec![]; }

        let cmds_ptr   = read_usize(data, 0).unwrap_or(0); 
        let cmds_len   = read_usize(data, cmds_ptr).unwrap_or(0);
        let cmds_start = cmds_ptr + 32;
        if cmds_start + cmds_len > data.len() { return vec![]; }
        let commands = &data[cmds_start..cmds_start + cmds_len];
        let inputs_ptr        = read_usize(data, 32).unwrap_or(0);
        let inputs_array_start= inputs_ptr + 32;

        let mut out = Vec::with_capacity(commands.len());
        for (idx, &cmd) in commands.iter().enumerate() {
            // Jump directly to the input slice for this command
            let off   = read_usize(data, inputs_array_start + idx * 32).unwrap_or(0);
            let start = inputs_ptr + off;
            
            if start + 32 > data.len() { continue; }
            let len = read_usize(data, start).unwrap_or(0);
            
            let slice_start = start + 32;
            if slice_start + len > data.len() { continue; }
            let inp = &data[slice_start..slice_start + len];
            if inp.len() < 160 { continue; }
            let recipient     = read_address(inp, 0).unwrap_or_default();
            let amount_in     = read_u256(inp, 32).unwrap_or_default();
            let amount_out_min= read_u256(inp, 64).unwrap_or_default();
            let path_off      = read_usize(inp, 96).unwrap_or(0);
            let path_len      = read_usize(inp, path_off).unwrap_or(0);
            let path_ptr      = path_off + 32;

            match cmd {
                0x00 if inp.len() >= path_ptr + path_len && path_len >= 43 => {
                    let pb = &inp[path_ptr..path_ptr + path_len];
                    let token_in  = Address::from_slice(&pb[0..20]);
                    let fee       = u32::from_be_bytes([0, pb[20], pb[21], pb[22]]);
                    let token_out = Address::from_slice(&pb[23..43]);
                    out.push(SwapInfo { dex: DexName::UniswapV3, router: to, token_in, token_out,
                                        amount_in, amount_out_min, to: recipient, fee: Some(fee), permit2_nonce: None });
                }
                0x08 if path_len >= 2 && inp.len() >= path_ptr + path_len * 32 => {
                    let token_in  = read_address(inp, path_ptr).unwrap_or_default();
                    let token_out = read_address(inp, path_ptr + (path_len - 1) * 32).unwrap_or_default();
                    out.push(SwapInfo { dex: DexName::UniswapV2, router: to, token_in, token_out,
                                        amount_in, amount_out_min, to: recipient, fee: None, permit2_nonce: None });
                }
                _ => {}
            }
        }
        out
    }
}
