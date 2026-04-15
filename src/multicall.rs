use alloy_primitives::Address;
use bytes::Bytes;

use crate::models::SwapInfo;
use crate::universal_decoder::{DecodeTx, UniversalDecoder};
use crate::utils::read_usize;

/// Multicall decoder — unwraps aggregate3 calldata and decodes each inner call.
#[derive(Default)]
pub struct Multicall {
    decoder: UniversalDecoder,
}

impl Multicall {
    pub fn new() -> Self { Self::default() }

    pub fn decode_aggregate(&self, data: Bytes, to: Address) -> Vec<SwapInfo> {
        if data.len() < 4 { return vec![]; }
        let raw = &data[4..];
        let array_loc = read_usize(raw, 0).unwrap_or(0);
        let array_len = read_usize(raw, array_loc + 32).unwrap_or(0);
        let mut out = Vec::new();
        for i in 0..array_len {
            let call_off    = array_loc + 32 + i * 32;
            let bytes_loc   = read_usize(raw, call_off).unwrap_or(0);
            let bytes_len   = read_usize(raw, bytes_loc).unwrap_or(0);
            let bytes_start = bytes_loc + 32;
            if bytes_start + bytes_len > raw.len() || bytes_len < 4 { continue; }
            
            let tx = DecodeTx {
                to:    Some(to),
                input: data.slice(bytes_start + 4..bytes_start + 4 + bytes_len), // Zero-copy slice
                ..Default::default()
            };
            out.extend(self.decoder.decode(&tx));
        }
        out
    }
}
