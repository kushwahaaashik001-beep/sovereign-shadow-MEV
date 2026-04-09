use ethers::types::{Address, U256, Bytes, H256, Chain};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use crate::utils::FailureType;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Selector(pub [u8; 4]);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum DexName {
    UniswapV2,
    UniswapV3,
    SushiSwap,
    Aerodrome,
    BaseSwap,
    PancakeSwap,
    Maverick,
    Permit2,
    CowSwap,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DexType {
    UniswapV2 = 0,
    UniswapV3 = 1,
    MaverickV2 = 2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hop {
    pub pool: Address,
    pub pool_address: Address,
    pub pool_address_label: Option<String>,
    pub token_in: Address,
    pub token_out: Address,
    pub dex_type: DexType,
    pub dex_name: DexName,
    pub zero_for_one: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Path {
    pub hops: Vec<Hop>,
    pub hash: H256,
    pub total_gas: u64,
}

impl Path {
    pub fn new(hops: &[Hop], total_gas: u64, _unused: u64) -> Self {
        Self {
            hops: hops.to_vec(),
            hash: H256::zero(),
            total_gas,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbitragePath {
    pub hops: Vec<Hop>,
    pub loan_token: Address,
    pub loan_amount: U256,
}

impl ArbitragePath {
    /// Ghost Protocol Encoding (Pillar F & G)
    /// Packs the path into the 42-byte format expected by the Yul Executor:
    /// [num_hops(1)] + [pool(20) | token_out(20) | dex_type(1) | zero_for_one(1)] * hops
    pub fn encode_ghost(&self) -> Bytes {
        let mut encoded = Vec::with_capacity(1 + self.hops.len() * 42);
        
        // First byte: Number of hops
        encoded.push(self.hops.len() as u8);

        for hop in &self.hops {
            // 1. Pool Address (20 bytes)
            encoded.extend_from_slice(hop.pool.as_bytes());
            
            // 2. Token Out Address (20 bytes) - Used for Ghost payment tracking
            encoded.extend_from_slice(hop.token_out.as_bytes());
            
            // 3. Flags (DexType & ZeroForOne)
            // Offset 40: dexType (0 for V2, 1 for V3)
            encoded.push(hop.dex_type.clone() as u8);
            
            // Offset 41: zeroForOne (1 if true, 0 if false)
            encoded.push(if hop.zero_for_one { 1 } else { 0 });
        }

        Bytes::from(encoded)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Opportunity {
    pub id: String,
    pub path: Arc<Path>,
    pub expected_profit: U256,
    pub gas_cost: U256,
    pub gas_estimate: U256,
    pub base_fee: U256,
    pub priority_fee: U256,
    pub input_token: Address,
    pub input_amount: U256,
    pub profit_details: Option<ProfitDetails>,
    pub chain: Chain,
    pub trigger_gas_price: Option<U256>,
    pub trigger_sender: Option<Address>,
    pub success_prob: u32,
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
    ProviderError(#[from] ethers::providers::ProviderError),
    #[error("Wallet error: {0}")]
    WalletError(#[from] ethers::signers::WalletError),
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
    #[error("Http error: {0}")]
    Http(String),
    #[error("Other error: {0}")]
    Other(String),
}

impl MEVError {
    pub fn failure_type(&self) -> FailureType {
        match self {
            MEVError::SimulationFailed(_) => FailureType::Revert,
            MEVError::HoneypotDetected(_) => FailureType::Other,
            _ => FailureType::Other,
        }
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
    pub fn address(&self, _chain: &Chain) -> Result<Address, MEVError> {
        Ok(Address::zero())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

pub const TOKEN_WETH: Address = ethers::types::H160([0x42, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x06]);