//! Shared utilities for the Sovereign Shadow engine.

use crate::models::MEVError;
use ethers::{
    prelude::*,
    providers::{Http, Provider},
    types::{Address, U256, transaction::eip2718::TypedTransaction, Chain},
    utils::keccak256,
    abi::Token,
};
use arc_swap::ArcSwap;
use std::{str::FromStr, fs::{self, OpenOptions, File}, io::{Write, BufRead, BufReader}};
use std::sync::{Arc, atomic::{AtomicU64, AtomicBool, Ordering}};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::warn;

// -----------------------------------------------------------------------------
// Circuit Breaker (pause on gas price spikes or consecutive failures)
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
    last_latency_ms: AtomicU64, // Pillar V: Latency awareness
    current_balance: ArcSwap<U256>, // Pillar V: Balance awareness
    manual_kill_switch: AtomicBool, // Pillar V: Dynamic Kill-Switch
    sequencer_stalled: AtomicBool,  // Pillar V: Sequencer Health Guard
}

impl CircuitBreaker {
    pub fn new(max_failures: u64, base_cooldown_secs: u64) -> Self {
        Self {
            failure_counts: [
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
            ],
            last_failure_time: AtomicU64::new(0),
            base_cooldown_secs,
            max_failures,
            last_latency_ms: AtomicU64::new(0),
            current_balance: ArcSwap::from_pointee(U256::zero()),
            manual_kill_switch: AtomicBool::new(false),
            sequencer_stalled: AtomicBool::new(false),
        }
    }

    pub fn is_open(&self) -> bool {
        // Pillar V: Master Kill-Switch (Immediate Veto)
        if self.manual_kill_switch.load(Ordering::SeqCst) {
            warn!("🛑 [VETO] Manual Kill-Switch activated. Watch-Only mode.");
            return true;
        }

        // Pillar V: Sequencer Stall Protection
        if self.sequencer_stalled.load(Ordering::Relaxed) {
            warn!("🛑 [VETO] Sequencer Stall detected. Aborting execution.");
            return true;
        }

        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

        // Pillar V: Latency Veto (Watch-Only Trigger)
        let latency = self.last_latency_ms.load(Ordering::Relaxed);
        if latency > crate::constants::MAX_BUILDER_LATENCY_MS && latency != 0 {
            warn!("🛡️ [VETO] High RPC latency: {}ms. Entering Watch-Only mode.", latency);
            return true;
        }

        // Pillar V: Balance Veto (Zero-Capital Protection)
        let balance = **self.current_balance.load();
        if balance < U256::from(crate::constants::MIN_SEARCHER_BALANCE_WEI) && !balance.is_zero() {
            warn!("🛡️ [VETO] Low balance: {} wei. Execution disabled.", balance);
            return true;
        }

        let last = self.last_failure_time.load(Ordering::Relaxed);
        let total: u64 = self.failure_counts.iter().map(|c| c.load(Ordering::Relaxed)).sum();
        if total < self.max_failures {
            return false;
        }
        // Cooldown increases with OutOfGas failures, as they are more indicative of a systemic issue.
        let cooldown = self.base_cooldown_secs * (1 + self.failure_counts[FailureType::OutOfGas as usize].load(Ordering::Relaxed));
        now.saturating_sub(last) < cooldown
    }

    /// Pillar V: Sequencer Drift Monitor
    /// Compares current system time with the latest block timestamp to detect L2 sequencer lag.
    pub fn record_sequencer_drift(&self, block_timestamp: u64) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let drift = now.saturating_sub(block_timestamp);
        
        if drift > crate::constants::MAX_NODE_LAG_SECONDS {
            if !self.sequencer_stalled.swap(true, Ordering::Relaxed) {
                warn!("⚠️ [SEQUENCER LAG] Drift detected: {}s. Vetoing execution.", drift);
            }
        } else {
            self.sequencer_stalled.store(false, Ordering::Relaxed);
        }
    }

    pub fn trigger_kill_switch(&self) {
        self.manual_kill_switch.store(true, Ordering::SeqCst);
    }

    pub fn reset_kill_switch(&self) {
        self.manual_kill_switch.store(false, Ordering::SeqCst);
    }

    pub fn record_failure(&self, ftype: FailureType) {
        let idx = ftype as usize;
        self.failure_counts[idx].fetch_add(1, Ordering::Relaxed);
        self.last_failure_time.store(
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
            Ordering::Relaxed,
        );
    }

    pub fn record_latency(&self, ms: u64) {
        self.last_latency_ms.store(ms, Ordering::Relaxed);
    }

    /// [PILLAR V] Returns the cached wallet balance.
    pub fn get_cached_balance(&self) -> U256 {
        **self.current_balance.load()
    }

    pub fn update_balance(&self, balance: U256) {
        self.current_balance.store(Arc::new(balance));
    }

    pub fn record_success(&self) {
        for c in &self.failure_counts {
            c.store(0, Ordering::Relaxed);
        }
    }
}

// -----------------------------------------------------------------------------
// Telegram Autonomous Notifier (Pillar R - Shadow Simulation)
// -----------------------------------------------------------------------------
pub fn send_telegram_msg(msg: &str) {
    let token = crate::constants::TELEGRAM_BOT_TOKEN;
    let chat_id = crate::constants::TELEGRAM_CHAT_ID;
    
    if token == "YOUR_BOT_TOKEN" || chat_id == "YOUR_CHAT_ID" {
        return;
    }

    let msg_owned = msg.to_string();

    // Fire and forget to avoid blocking the hot-path
    tokio::spawn(async move {
        let url = format!("https://api.telegram.org/bot{}/sendMessage", token);
        let client = reqwest::Client::new();
        let payload = serde_json::json!({
            "chat_id": chat_id,
            "text": msg_owned,
            "parse_mode": "Markdown"
        });
        if let Err(e) = client.post(url).json(&payload).send().await {
            warn!("⚠️ [TELEGRAM] Failed to send notification: {}", e);
        }
    });
}

// -----------------------------------------------------------------------------
// Auditor System (Pillar R - Decision Logging & Self-Maintenance)
// -----------------------------------------------------------------------------
pub fn audit_log(pillar: &str, msg: &str) {
    let dir = "logs";
    let path = "logs/rejection_auditor.log";
    
    // Ensure directory exists
    if let Err(e) = fs::create_dir_all(dir) {
        warn!("⚠️ [AUDITOR] Could not create logs directory: {}", e);
        return;
    }

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        // Format: [Timestamp] [Pillar] Message
        let _ = writeln!(file, "[{}] [{}] {}", now, pillar, msg);
    }
}

/// [PILLAR V] Automatically truncates logs to prevent storage bloat.
/// Keeps only the last 1000 decision points.
pub fn cleanup_auditor_logs() {
    let path = "logs/rejection_auditor.log";
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return, // File doesn't exist yet
    };

    let reader = BufReader::new(file);
    let lines: Vec<String> = reader.lines().filter_map(|l| l.ok()).collect();
    
    if lines.len() > 2000 {
        if let Ok(mut file) = File::create(path) {
            // Keep the most recent 1000 lines
            for line in lines.iter().skip(lines.len() - 1000) {
                let _ = writeln!(file, "{}", line);
            }
            warn!("🧹 [AUDITOR] Log maintenance complete. Rotated 1000+ lines.");
        }
    }
}

// -----------------------------------------------------------------------------
// L1 Data Fee Calculator for L2s
// -----------------------------------------------------------------------------
#[derive(Clone)]
pub struct L1DataFeeCalculator {
    provider: Arc<Provider<Http>>,
}

impl L1DataFeeCalculator {
    pub fn new(provider: Arc<Provider<Http>>) -> Self {
        Self { provider }
    }

    pub async fn estimate_l1_fee(&self, chain: Chain, tx_data: &[u8]) -> Result<U256, MEVError> {
        match chain {
            Chain::Arbitrum => {
                // On Arbitrum, we use the NodeInterface precompile to get a more accurate estimate.
                let node_interface = Address::from_str("0x00000000000000000000000000000000000000C8").unwrap();
                let selector = &keccak256(b"gasEstimateL1Component(address,bool,bytes)")[..4];
                let mut call_data = selector.to_vec();
                // We need a `to` address for the estimate. We can use a zero address as a placeholder.
                call_data.extend_from_slice(&ethers::abi::encode(&[
                    Token::Address(Address::zero()),
                    Token::Bool(false),
                    Token::Bytes(tx_data.to_vec()),
                ]));
                let tx: TypedTransaction = TransactionRequest::new().to(node_interface).data(call_data).into();
                let result = self.provider.call(&tx, None).await?;
                if result.len() >= 64 {
                    // The fee is in the second 32-byte word of the return data.
                    let l1_fee = U256::from_big_endian(&result[32..64]);
                    Ok(l1_fee)
                } else {
                    Err(MEVError::Other("Invalid Arbitrum NodeInterface response".into()))
                }
            }
            Chain::Optimism | Chain::Base => {
                // On Optimism and Base, we use the GasPriceOracle precompile.
                let oracle = Address::from_str("0x420000000000000000000000000000000000000F").unwrap();
                let selector = &keccak256(b"getL1Fee(bytes)")[..4];
                let mut call_data = selector.to_vec();
                call_data.extend_from_slice(&ethers::abi::encode(&[Token::Bytes(tx_data.to_vec())]));
                let tx: TypedTransaction = TransactionRequest::new().to(oracle).data(call_data).into();
                let result = self.provider.call(&tx, None).await?;
                Ok(U256::from_big_endian(&result))
            }
            // For other chains, or as a fallback, we can use a simpler estimation.
            // This is a rough estimate and should be refined.
            _ => {
                let l1_base_fee = self.provider.get_gas_price().await.unwrap_or_default();
                // A very rough approximation: 16 gas per byte for non-zeros, 4 for zeros.
                // We'll just use 16 for a pessimistic estimate.
                Ok(l1_base_fee * U256::from(tx_data.len() * 16))
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Zero-Copy ABI Readers (Pillar A/F Hot-Path Optimized)
// -----------------------------------------------------------------------------
#[inline(always)]
pub fn read_u256(data: &[u8], offset: usize) -> Option<U256> {
    let end = offset + 32;
    if end > data.len() {
        return None;
    }
    // Direct slice, no temp array allocation
    Some(U256::from_big_endian(&data[offset..end]))
}

#[inline(always)]
pub fn read_address(data: &[u8], offset: usize) -> Option<Address> {
    let addr_start = offset + 12;
    let end = addr_start + 20;
    if end > data.len() {
        return None;
    }
    Some(Address::from_slice(&data[addr_start..end]))
}

#[inline(always)]
pub fn read_usize(data: &[u8], offset: usize) -> Option<usize> {
    // ABI slots are 32 bytes. We read the last 8 bytes for usize.
    // Using .get() prevents panics, and try_into is zero-cost.
    data.get(offset + 24..offset + 32)?
        .try_into()
        .ok()
        .map(usize::from_be_bytes)
}
