// src/auditor.rs
// 🛡️ THE SOVEREIGN SHADOW: BLACK BOX EXECUTION AUDITOR

use ethers::types::{U256, H256, Address};
use std::fs::{OpenOptions, create_dir_all};
use std::io::Write;
use std::path::Path;
use crate::utils::send_telegram_msg;
use std::time::{SystemTime, UNIX_EPOCH};
use crate::models::Opportunity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStatus {
    Success,
    Reverted,
    SimFailed,
    BudgetExceeded,
}

impl std::fmt::Display for ExecutionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutionStatus::Success => write!(f, "SUCCESS"),
            ExecutionStatus::Reverted => write!(f, "REVERTED"),
            ExecutionStatus::SimFailed => write!(f, "SIM_FAILED"),
            ExecutionStatus::BudgetExceeded => write!(f, "BUDGET_EXCEEDED"),
        }
    }
}

impl From<crate::utils::FailureType> for ExecutionStatus {
    fn from(ft: crate::utils::FailureType) -> Self {
        match ft {
            crate::utils::FailureType::OutOfGas => ExecutionStatus::BudgetExceeded, // Closest match
            crate::utils::FailureType::Slippage => ExecutionStatus::Reverted, // Slippage often leads to revert
            crate::utils::FailureType::Revert => ExecutionStatus::Reverted,
            crate::utils::FailureType::Other => ExecutionStatus::SimFailed, // General failure, assume sim failed
        }
    }
}

pub struct TradeLog {
    pub timestamp: u64,
    pub tx_hash: H256,
    pub target_path: String,
    pub expected_profit_wei: U256,
    pub actual_profit_received_wei: U256,
    pub l2_gas_used: u64,
    pub l1_data_fee_wei: U256,
    pub status: ExecutionStatus,
    pub revert_reason: String,
}

impl TradeLog {
    pub fn new(opp: &Opportunity, l2_gas_used: u64, l1_data_fee_wei: U256) -> Self {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        
        let mut path_elements = Vec::new();
        if !opp.path.hops.is_empty() {
            for hop in &opp.path.hops {
                path_elements.push(get_symbol(hop.token_in));
            }
            path_elements.push(get_symbol(opp.path.hops.last().unwrap().token_out));
        }
        
        Self {
            timestamp,
            tx_hash: H256::zero(),
            target_path: path_elements.join("->"),
            expected_profit_wei: opp.expected_profit,
            actual_profit_received_wei: U256::zero(), // Will be updated on success
            l2_gas_used,
            l1_data_fee_wei,
            status: ExecutionStatus::SimFailed,
            revert_reason: String::new(),
        }
    }
}

fn get_symbol(addr: Address) -> String {
    if addr == crate::constants::TOKEN_WETH { "ETH".to_string() }
    else if addr == crate::constants::TOKEN_USDC { "USDC".to_string() }
    else if addr == crate::constants::TOKEN_DAI { "DAI".to_string() }
    else { 
        let s = format!("{:?}", addr);
        if s.len() > 6 { s[2..6].to_string() } else { "ALT".to_string() }
    }
}

pub fn save_audit_entry(log: &TradeLog) {
    let dir = "logs";
    let file_path = "logs/shadow_audit.csv";

    if !Path::new(dir).exists() {
        let _ = create_dir_all(dir);
    }

    let file_exists = Path::new(file_path).exists();
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(file_path) {
        if !file_exists {
            let _ = writeln!(file, "timestamp,tx_hash,target_path,expected_profit_wei,actual_profit_received_wei,l2_gas_used,l1_data_fee_wei,status,revert_reason");
        }

        let _ = writeln!(
            file,
            "{},{:?},{},{},{},{},{},{},{}",
            log.timestamp, log.tx_hash, log.target_path,
            log.expected_profit_wei, log.actual_profit_received_wei,
            log.l2_gas_used, log.l1_data_fee_wei, log.status,
            log.revert_reason.replace(",", ";")
        );
    }

    // Clean Terminal Output
    println!("\n📊 [AUDIT] Path: {} | Net: +{} Wei | Gas: {} | Status: {}", 
        log.target_path, log.actual_profit_received_wei, log.l2_gas_used, log.status);

    // Pillar R: Telegram Notification for successful trades
    let profit_eth = log.actual_profit_received_wei.as_u128() as f64 / 1e18;
    let msg = format!("✅ *Trade SUCCESS!* ✨\n\n💰 Profit: `{:.6} ETH`\n🛣️ Path: `{}`\n🔗 Tx: `https://basescan.org/tx/{:?}`", profit_eth, log.target_path, log.tx_hash);
    send_telegram_msg(&msg);
}

/// Pillar F: Missed Opportunity Analysis
/// Logs why a specific path was rejected during the detection phase.
pub fn log_rejection(path: String, reason: &str, expected_profit: U256, total_cost: U256) {
    let dir = "logs";
    let file_path = "logs/rejections.csv";

    if !std::path::Path::new(dir).exists() {
        let _ = create_dir_all(dir);
    }

    let file_exists = std::path::Path::new(file_path).exists();
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(file_path) {
        if !file_exists {
            let _ = writeln!(file, "timestamp,path,reason,gross_profit_wei,total_cost_wei");
        }

        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let _ = writeln!(
            file,
            "{},{},{},{},{}",
            timestamp, path, reason, expected_profit, total_cost
        );
    }

    // Pillar R: Telegram Notification for rejected opportunities
    let profit_eth = expected_profit.as_u128() as f64 / 1e18;
    let msg = format!("❌ *Trade REJECTED!* 🚫\n\nReason: `{}`\n🛣️ Path: `{}`\n💰 Est. Profit: `{:.6} ETH`", reason, path, profit_eth);
    send_telegram_msg(&msg);
}