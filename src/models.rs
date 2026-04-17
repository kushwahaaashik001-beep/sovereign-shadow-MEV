pub use alloy_primitives::{Address, U256, B256, FixedBytes, Bytes, Uint};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[repr(u64)]
pub enum Chain {
    #[default]
    Base = 8453,
}

impl Chain {
    pub fn try_from_id(id: u64) -> Option<Self> {
        if id == 8453 { Some(Chain::Base) } else { None }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Selector(pub [u8; 4]);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub enum DexName {
    #[default]
    UniswapV2,
    UniswapV3,
    SushiSwap,
    Aerodrome,
    BaseSwap,
    PancakeSwap,
    Maverick,
    Permit2,
    CowSwap,
    UniswapX,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum DexType {
    #[default]
    UniswapV2 = 0,
    UniswapV3 = 1,
    MaverickV2 = 2,
    Aerodrome = 3,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Hop {
    pub pool: Address,
    pub pool_address: Address,
    pub pool_address_label: Option<String>,
    pub token_in: Address,
    pub token_out: Address,
    pub dex_type: DexType,
    pub dex_name: DexName,
    pub zero_for_one: bool,
    pub is_stable: bool,
    pub fee: Option<u32>,
    pub static_calldata: Bytes,
    pub gas_cost: U256,
    pub id: [u8; 32],
    pub success_prob: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PoolKey {
    pub pool: Address,
}

#[derive(Debug, Clone)]
pub struct SwapInfo {
    pub dex: DexName,
    pub router: Address,
    pub token_in: Address,
    pub token_out: Address,
    pub amount_in: U256,
    pub amount_out_min: U256,
    pub to: Address,
    pub fee: Option<u32>,
    pub permit2_nonce: Option<U256>,
}

impl SwapInfo {
    pub fn is_tracked(&self, tracked: &rustc_hash::FxHashSet<PoolKey>) -> bool {
        tracked.contains(&PoolKey { pool: self.router })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Path {
    pub hops: Vec<Hop>,
    pub hash: B256,
    pub total_gas: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MempoolTx {
    pub data: Bytes,
    pub hash: B256,
    pub to: Option<Address>,
}

impl Path {
    pub fn new(hops: &[Hop], total_gas: u64) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for hop in hops {
            std::hash::Hash::hash(&hop.pool_address, &mut hasher);
            std::hash::Hash::hash(&hop.token_out, &mut hasher);
        }
        let h_val = std::hash::Hasher::finish(&hasher);
        let mut hash_bytes = [0u8; 32];
        hash_bytes[24..32].copy_from_slice(&h_val.to_be_bytes());
        Self { hops: hops.to_vec(), hash: B256::from(hash_bytes), total_gas }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbitragePath {
    pub hops: Vec<Hop>,
    pub loans: Vec<(Address, U256)>,
    pub lender: Lender,
}

impl ArbitragePath {
    pub fn encode_ghost_multi(&self, loans: Vec<(Address, U256)>, min_profit: U256) -> Bytes {
        let mut encoded = Vec::new();
        encoded.extend_from_slice(&min_profit.to_be_bytes::<32>());
        encoded.push(self.lender.id()); // Prefix with lender ID
        encoded.push(loans.len() as u8);
        for (token, amount) in loans {
            encoded.extend_from_slice(token.as_slice());
            encoded.extend_from_slice(&amount.to_be_bytes::<32>());
        }
        encoded.push(self.hops.len() as u8);
        for hop in &self.hops {
            encoded.extend_from_slice(hop.pool.as_slice());
            encoded.extend_from_slice(hop.token_out.as_slice());
            encoded.push(hop.dex_type as u8);
            encoded.push(if hop.zero_for_one { 1 } else { 0 });
            encoded.push(if hop.is_stable { 1 } else { 0 }); // Byte 42: stability info
        }
        Bytes::from(encoded)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Bundle {
    pub transactions: Vec<Vec<u8>>,
    pub target_block: u64,
    pub bribe: U256,
}

impl Bundle {
    pub fn new() -> Self { Self::default() }
    pub fn set_bribe(&mut self, bribe: U256) {
        self.bribe = bribe;
    }
}

#[derive(Debug, Clone, Default)]
pub struct Opportunity {
    pub id:              String,
    pub path:            Arc<Path>,
    pub expected_profit: U256,
    pub gas_cost:        U256,
    pub gas_estimate:    U256,
    pub base_fee:        U256,
    pub priority_fee:    U256,
    pub input_token:     Address,
    pub input_amount:    U256,
    pub profit_details:  Option<ProfitDetails>,
    pub executor_address: Address,
    pub is_whale_trigger: bool,
    pub chain:           Chain,
    pub trigger_gas_price: Option<U256>,
    pub trigger_sender:  Option<Address>,
    pub pending_txs:     Vec<MempoolTx>,
    pub success_prob:    u32,
    pub static_calldata: Bytes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfitDetails {
    pub net_profit: U256,
    pub slippage: f64,
    pub gas_savings: U256,
}

#[derive(Error, Debug)]
pub enum MEVError {
    #[error("Simulation failed: {0}")]
    SimulationFailed(String),
    #[error("Honeypot detected: {0}")]
    HoneypotDetected(String),
    #[error("Circuit breaker open")]
    CircuitBreakerOpen,
    #[error("No relay accepted")]
    NoRelayAccepted,
    #[error("Provider error: {0}")]
    ProviderError(String),
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
    #[error("Http error: {0}")]
    Http(String),
    #[error("Other error: {0}")]
    Other(String),
}

impl MEVError {
    pub fn is_revert(&self) -> bool {
        matches!(self, MEVError::SimulationFailed(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Lender {
    Balancer,
    AaveV3,
    UniswapV2,
    UniswapV3,
    Aerodrome,
    MakerDAO,
    MorphoBlue,
    Curve,
}

impl Lender {
    pub fn id(&self) -> u8 {
        match self {
            Lender::Balancer => 0,
            Lender::AaveV3 => 1,
            _ => 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PoolEdge {
    pub pool_address: Address,
    pub dex_name: DexName,
    pub token_b: Address,
    pub fee: Option<u32>,
    pub liq_score: u64,
    pub static_calldata: Bytes,
    pub gas_cost: U256,
    pub id: [u8; 32],
    pub success_prob: u32,
}

pub const TOKEN_WETH: Address = alloy_primitives::address!("4200000000000000000000000000000000000006");
