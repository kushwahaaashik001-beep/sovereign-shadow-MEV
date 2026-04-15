//! Shared utilities for the Sovereign Shadow engine.
#![allow(dead_code)]

use crate::models::MEVError;
use alloy_primitives::{Address, U256};
use alloy::providers::RootProvider;
use alloy::transports::BoxTransport;
use alloy::sol;
use arc_swap::ArcSwap;
use std::{fs::{self, OpenOptions, File}, io::{Write, BufRead, BufReader}};
use std::sync::{Arc, atomic::{AtomicU64, AtomicBool, Ordering}};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::warn;

sol! {
    #[sol(rpc)]
    interface IGasPriceOracle {
        function getL1Fee(bytes memory data) external view returns (uint256);
        function baseFeeScalar() external view returns (uint32);
        function blobBaseFeeScalar() external view returns (uint32);
        function l1BaseFee() external view returns (uint256);
        function blobBaseFee() external view returns (uint256);
        function decimals() external view returns (uint256);
    }

    #[sol(rpc)]
    interface INodeInterface {
        function gasEstimateL1Component(address to, bool contractCustomData, bytes calldata data) external view returns (uint64 gasEstimateForL1, uint256 baseFee, uint256 l1BaseFee);
    }
}

// -----------------------------------------------------------------------------
// Circuit Breaker
// -----------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureType {
    OutOfGas,
    Slippage,
    Revert,
    Other,
}

pub struct CircuitBreaker {
    failure_counts: [AtomicU64; 4],
    last_failure_time: AtomicU64,
    base_cooldown_secs: u64,
    max_failures: u64,
    last_latency_ms: AtomicU64,
    current_balance: ArcSwap<U256>,
    manual_kill_switch: AtomicBool,
    pub atomic_shield_active: AtomicBool,
    sequencer_stalled: AtomicBool,
}

impl CircuitBreaker {
    pub fn new(max_failures: u64, base_cooldown_secs: u64) -> Self {
        Self {
            failure_counts: [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)],
            last_failure_time: AtomicU64::new(0),
            base_cooldown_secs,
            max_failures,
            last_latency_ms: AtomicU64::new(0),
            atomic_shield_active: AtomicBool::new(true),
            current_balance: ArcSwap::from_pointee(U256::ZERO),
            manual_kill_switch: AtomicBool::new(false),
            sequencer_stalled: AtomicBool::new(false),
        }
    }

    pub fn is_open(&self) -> bool {
        if self.manual_kill_switch.load(Ordering::SeqCst) { return true; }
        if self.sequencer_stalled.load(Ordering::Relaxed) { return true; }
        let latency = self.last_latency_ms.load(Ordering::Relaxed);
        if latency > crate::constants::MAX_BUILDER_LATENCY_MS && latency != 0 { return true; }
        let balance = **self.current_balance.load();
        if balance < U256::from(crate::constants::MIN_SEARCHER_BALANCE_WEI) && !balance.is_zero() { return true; }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let last = self.last_failure_time.load(Ordering::Relaxed);
        let total: u64 = self.failure_counts.iter().map(|c| c.load(Ordering::Relaxed)).sum();
        if total < self.max_failures { return false; }
        let cooldown = self.base_cooldown_secs * (1 + self.failure_counts[FailureType::OutOfGas as usize].load(Ordering::Relaxed));
        now.saturating_sub(last) < cooldown
    }

    pub fn record_sequencer_drift(&self, block_timestamp: u64) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let drift = now.saturating_sub(block_timestamp);
        if drift > crate::constants::MAX_NODE_LAG_SECONDS {
            if !self.sequencer_stalled.swap(true, Ordering::Relaxed) {
                warn!("⚠️ [SEQUENCER LAG] Drift: {}s", drift);
            }
        } else {
            self.sequencer_stalled.store(false, Ordering::Relaxed);
        }
    }

    pub fn trigger_kill_switch(&self) { self.manual_kill_switch.store(true, Ordering::SeqCst); }
    pub fn reset_kill_switch(&self) { self.manual_kill_switch.store(false, Ordering::SeqCst); }

    pub fn record_failure(&self, ftype: FailureType) {
        self.failure_counts[ftype as usize].fetch_add(1, Ordering::Relaxed);
        self.last_failure_time.store(
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
            Ordering::Relaxed,
        );

        // Pillar V: Automatic Veto - Emergency Shutdown
        let total_fails: u64 = self.failure_counts.iter().map(|c| c.load(Ordering::Relaxed)).sum();
        if total_fails > 20 { // Critical threshold for small budget
            crate::constants::GLOBAL_PAUSE.store(true, Ordering::SeqCst);
            warn!("🚨 [PILLAR V] SYSTEM VETO: Too many failures. Emergency Shutdown triggered.");
        }
    }

    pub fn record_latency(&self, ms: u64) { self.last_latency_ms.store(ms, Ordering::Relaxed); }
    pub fn get_cached_balance(&self) -> U256 { **self.current_balance.load() }
    pub fn update_balance(&self, balance: U256) { self.current_balance.store(Arc::new(balance)); }
    pub fn record_success(&self) { for c in &self.failure_counts { c.store(0, Ordering::Relaxed); } }
}

// -----------------------------------------------------------------------------
// L1DataFeeCalculator stub
// -----------------------------------------------------------------------------
#[derive(Clone)]
pub struct EcotoneScalars {
    pub base_fee_scalar: u32,
    pub blob_fee_scalar: u32,
    pub l1_base_fee: U256,
    pub l1_blob_fee: U256,
    pub decimals: U256,
    pub last_updated: u64,
}

pub struct L1DataFeeCalculator {
    provider: Arc<RootProvider<BoxTransport>>,
    scalars: ArcSwap<EcotoneScalars>,
}

impl L1DataFeeCalculator {
    pub fn new(provider: Arc<RootProvider<BoxTransport>>) -> Arc<Self> { 
        let scalars = ArcSwap::from_pointee(EcotoneScalars {
            base_fee_scalar: 0,
            blob_fee_scalar: 0,
            l1_base_fee: U256::ZERO,
            l1_blob_fee: U256::ZERO,
            decimals: U256::from(6),
            last_updated: 0,
        });
        Arc::new(Self { provider, scalars }) 
    }

    /// Pillar J: Refresh scalars from Oracle contract every 5 mins to save RPC calls.
    pub async fn refresh_scalars(&self, chain: crate::models::Chain) -> Result<(), MEVError> {
        if chain == crate::models::Chain::Arbitrum {
            let node_interface = INodeInterface::INodeInterfaceInstance::new(crate::constants::ARBITRUM_NODE_INTERFACE, self.provider.clone());
            if let Ok(res) = node_interface.gasEstimateL1Component(Address::ZERO, false, Vec::new().into()).call().await {
                let mut sc = (**self.scalars.load()).clone();
                sc.l1_base_fee = res.l1BaseFee;
                sc.last_updated = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                self.scalars.store(Arc::new(sc));
            }
        } else {
            let oracle = IGasPriceOracle::IGasPriceOracleInstance::new(crate::constants::OPTIMISM_GAS_ORACLE, self.provider.clone());
            let base_scalar = oracle.baseFeeScalar().call().await.map(|r| r._0).unwrap_or(0);
            let blob_scalar = oracle.blobBaseFeeScalar().call().await.map(|r| r._0).unwrap_or(0);
            let l1_base = oracle.l1BaseFee().call().await.map(|r| r._0).unwrap_or(U256::ZERO);
            let l1_blob = oracle.blobBaseFee().call().await.map(|r| r._0).unwrap_or(U256::ZERO);
            let decimals = oracle.decimals().call().await.map(|r| r._0).unwrap_or(U256::from(6));

            self.scalars.store(Arc::new(EcotoneScalars {
                base_fee_scalar: base_scalar,
                blob_fee_scalar: blob_scalar,
                l1_base_fee: l1_base,
                l1_blob_fee: l1_blob,
                decimals,
                last_updated: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
            }));
        }
        Ok(())
    }

    pub async fn estimate_l1_fee(&self, chain: crate::models::Chain, tx_data: &[u8]) -> Result<U256, MEVError> {
        if !matches!(chain, crate::models::Chain::Base | crate::models::Chain::Optimism | crate::models::Chain::Arbitrum) {
            return Ok(U256::ZERO);
        }

        if chain == crate::models::Chain::Arbitrum {
            let node_interface = INodeInterface::INodeInterfaceInstance::new(crate::constants::ARBITRUM_NODE_INTERFACE, self.provider.clone());
            if let Ok(res) = node_interface.gasEstimateL1Component(Address::ZERO, false, tx_data.to_vec().into()).call().await {
                return Ok(U256::from(res.gasEstimateForL1) * res.l1BaseFee);
            }
        }

        let sc = self.scalars.load();
        
        if sc.last_updated == 0 {
             return Err(MEVError::Other("L1 Scalars not yet initialized".into()));
        }

        // Pillar J: Ultra-Precise L1 Calldata Cost (Base Ecotone Tuning)
        // A standard EIP-1559 Tx on Base has ~68 bytes of RLP overhead + 65 bytes for Sig.
        // Total fixed overhead ~133 bytes.
        let mut calldata_gas = 133u64 * 16; 
        for &b in tx_data {
            calldata_gas += if b == 0 { 4 } else { 16 };
        }
        
        // Ecotone Formula: L1_Fee = (16*baseScalar*l1Base + blobScalar*l1Blob) * calldataGas / (16 * 10^decimals)
        let scaled_base = U256::from(16) * U256::from(sc.base_fee_scalar) * sc.l1_base_fee;
        let scaled_blob = U256::from(sc.blob_fee_scalar) * sc.l1_blob_fee;
        let total_l1_gas_price = scaled_base + scaled_blob;
        
        // Pillar J: Precision Ecotone Divisor
        let dec_val = sc.decimals.to::<u64>(); // Convert to u64 for U256::pow
        let divisor = U256::from(16) * U256::from(10u128.pow(dec_val as u32));
        
        if divisor.is_zero() { return Ok(U256::ZERO); }

        let fee = (total_l1_gas_price * U256::from(calldata_gas)) / divisor;
        
        // Add 10% safety buffer
        Ok((fee * U256::from(110)) / U256::from(100))
    }
}

// -----------------------------------------------------------------------------
// Auditor
// -----------------------------------------------------------------------------
pub fn audit_log(pillar: &str, msg: &str) {
    let dir = "logs";
    let path = "logs/rejection_auditor.log";
    if let Err(e) = fs::create_dir_all(dir) { warn!("⚠️ [AUDITOR] {}", e); return; }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let _ = writeln!(file, "[{}] [{}] {}", now, pillar, msg);
    }
}

pub fn cleanup_auditor_logs() {
    let path = "logs/rejection_auditor.log";
    let file = match File::open(path) { Ok(f) => f, Err(_) => return };
    let reader = BufReader::new(file);
    let lines: Vec<String> = reader.lines().filter_map(|l| l.ok()).collect();
    if lines.len() > 2000 {
        if let Ok(mut file) = File::create(path) {
            for line in lines.iter().skip(lines.len() - 1000) { let _ = writeln!(file, "{}", line); }
            warn!("🧹 [AUDITOR] Log rotated.");
        }
    }
}

// -----------------------------------------------------------------------------
// Zero-Copy ABI Readers
// -----------------------------------------------------------------------------
#[inline(always)]
pub fn read_u256(data: &[u8], offset: usize) -> Option<U256> {
    let end = offset + 32;
    if end > data.len() { return None; }
    Some(U256::from_be_slice(&data[offset..end]))
}

#[inline(always)]
pub fn read_address(data: &[u8], offset: usize) -> Option<Address> {
    let addr_start = offset + 12;
    let end = addr_start + 20;
    if end > data.len() { return None; }
    Some(Address::from_slice(&data[addr_start..end]))
}

#[inline(always)]
pub fn read_usize(data: &[u8], offset: usize) -> Option<usize> {
    data.get(offset + 24..offset + 32)?
        .try_into().ok()
        .map(usize::from_be_bytes)
}

pub fn slice_v3_path(data: &[u8]) -> Option<(Address, u32, Address)> {
    if data.len() < 43 { return None; }
    let a = Address::from_slice(&data[0..20]);
    let fee = u32::from_be_bytes([0, data[20], data[21], data[22]]);
    let b = Address::from_slice(&data[23..43]);
    Some((a, fee, b))
}

pub fn fast_decode_v3_path(path: &[u8]) -> Vec<(Address, u32, Address)> {
    let mut decoded = Vec::with_capacity(3);
    if path.len() < 43 { return decoded; }
    
    let mut i = 0;
    while i + 43 <= path.len() {
        let token_a = Address::from_slice(&path[i..i+20]);
        let fee = u32::from_be_bytes([0, path[i+20], path[i+21], path[i+22]]);
        let token_b = Address::from_slice(&path[i+23..i+43]);
        decoded.push((token_a, fee, token_b));
        
        // V3 Paths are packed as [addr20, fee3, addr20, fee3, addr20]
        // We hop 23 bytes (addr20 + fee3) to get the next pair
        i += 23;
    }
    decoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::info;
    use alloy::providers::ProviderBuilder;
    use crate::models::Chain;

    #[test]
    fn test_l1_fee_estimation_logic() {
        let provider = Arc::new(ProviderBuilder::new().on_http("http://localhost:8545".parse().unwrap()).boxed());
        let calc = L1DataFeeCalculator::new(provider);
        
        // Manually inject Ecotone scalars for testing
        calc.scalars.store(Arc::new(EcotoneScalars {
            base_fee_scalar: 1360,
            blob_fee_scalar: 1360,
            l1_base_fee: U256::from(1_000_000_000u64), // 1 gwei
            l1_blob_fee: U256::from(1_000_000u64),
            decimals: U256::from(6),
            last_updated: 1234567,
        }));

        let dummy_tx = vec![0xaa, 0xbb, 0x00, 0x11]; // Some non-zero, one zero
        let fee = tokio::runtime::Runtime::new().unwrap().block_on(async {
            calc.estimate_l1_fee(Chain::Base, &dummy_tx).await.unwrap()
        });

        assert!(fee > U256::ZERO);
        info!("Tested L1 Fee: {} wei", fee);
    }

    #[test]
    fn test_v3_path_decoding() {
        let path = alloy::hex::decode("42000000000000000000000000000000000000060001f4833589fcd6edb6e08f4c7c32d4f71b54bda02913").unwrap();
        let decoded = fast_decode_v3_path(&path);
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].1, 500); // 500 bps fee
    }
}
