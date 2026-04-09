use crate::{
    models::SwapInfo,
    universal_decoder::UniversalDecoder,
    utils::read_usize,
};
use ethers::types::{Address, Bytes, Transaction};

#[derive(Debug, Clone)]
pub struct Multicall {
    decoder: UniversalDecoder,
}

impl Default for Multicall {
    fn default() -> Self { Self::new() }
}

impl Multicall {
    pub fn new() -> Self { Self { decoder: UniversalDecoder::new() } }

    pub async fn decode_aggregate(&self, data: &[u8], to: Address) -> Vec<Result<SwapInfo, ()>> {
        if data.len() < 4 { return vec![]; }
        let data = &data[4..];
        let array_loc = read_usize(data, 0).unwrap_or(0);
        let array_len = read_usize(data, array_loc + 32).unwrap_or(0);
        let mut results = Vec::new();
        for i in 0..array_len {
            let call_off = array_loc + 32 + i * 32;
            let bytes_loc = read_usize(data, call_off).unwrap_or(0);
            let bytes_len = read_usize(data, bytes_loc).unwrap_or(0);
            let bytes_start = bytes_loc + 32;
            if bytes_start + bytes_len > data.len() || bytes_len < 4 { continue; }
            let call_data = &data[bytes_start..bytes_start + bytes_len];
            let dummy_tx = Transaction {
                input: Bytes::from(call_data.to_vec()),
                to: Some(to),
                ..Default::default()
            };
            results.extend(self.decoder.decode_transaction_deep(&dummy_tx));
        }
        results
    }
}
