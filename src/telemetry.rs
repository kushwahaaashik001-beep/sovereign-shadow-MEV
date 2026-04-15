use std::env;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use reqwest::Client;
use serde_json::json;

#[derive(Debug, Clone)]
pub enum TelemetryEvent {
    OpportunityFound { path: String, est_profit: f64 },
    SimulationPassed { profit: f64, gas_used: u64 },
    SimulationFailed { reason: String },
    Rejection { reason: String, profit: f64 },
    ExecutionStarted { tx_hash: String },
    ExecutionSuccess { tx_hash: String, net_profit: f64 },
    ExecutionFailed { error: String },
    Heartbeat { balance: f64, block: u64 },
}

pub struct TelemetryHandle {
    sender: mpsc::UnboundedSender<TelemetryEvent>,
}

impl TelemetryHandle {
    pub fn new(sender: mpsc::UnboundedSender<TelemetryEvent>) -> Self {
        Self { sender }
    }

    pub fn send(&self, event: TelemetryEvent) {
        let _ = self.sender.send(event);
    }
}

pub async fn run_telemetry_loop(mut receiver: mpsc::UnboundedReceiver<TelemetryEvent>) {
    let token = env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default();
    let chat_id = env::var("TELEGRAM_CHAT_ID").unwrap_or_default();

    if token.is_empty() || chat_id.is_empty() {
        warn!("⚠️ [TELEMETRY] Telegram credentials not set in .env. Dashboard reporting is disabled.");
        return;
    }

    let client = Client::new();
    let url = format!("https://api.telegram.org/bot{}/sendMessage", token);

    info!("📢 [TELEMETRY] Dashboard reporting service started");

    while let Some(event) = receiver.recv().await {
        let message = match event {
            TelemetryEvent::OpportunityFound { path, est_profit } => {
                format!("🔍 <b>Opp Found!</b>\nPath: <code>{}</code>\nEst. Profit: <b>{:.6} ETH</b>", path, est_profit)
            }
            TelemetryEvent::SimulationPassed { profit, gas_used } => {
                format!("🛡️ <b>Sim Success</b>\nProfit: <code>{:.6} ETH</code>\nGas: <code>{}</code>", profit, gas_used)
            }
            TelemetryEvent::SimulationFailed { reason } => {
                format!("❌ <b>Sim Failed</b>\nReason: <i>{}</i>", reason)
            }
            TelemetryEvent::Rejection { reason, profit } => {
                format!("⚠️ <b>Rejected</b>\nReason: {}\nProfit lost: {:.4} ETH", reason, profit)
            }
            TelemetryEvent::ExecutionStarted { tx_hash } => {
                format!("🚀 <b>Firing Transaction!</b>\nHash: <a href='https://basescan.org/tx/{}'>View</a>", tx_hash)
            }
            TelemetryEvent::ExecutionSuccess { tx_hash, net_profit } => {
                format!("💰 <b>PROFIT HARVESTED!</b>\nNet: <b>{:.6} ETH</b>\nTx: <code>{}</code>", net_profit, tx_hash)
            }
            TelemetryEvent::ExecutionFailed { error } => {
                format!("💀 <b>Execution REVERTED</b>\nError: <code>{}</code>", error)
            }
            TelemetryEvent::Heartbeat { balance, block } => {
                format!("💓 <b>Bot Alive</b>\nBlock: <code>{}</code>\nWallet: <code>{:.4} ETH</code>", block, balance)
            }
        };

        let payload = json!({
            "chat_id": chat_id,
            "text": message,
            "parse_mode": "HTML",
            "disable_web_page_preview": true
        });

        if let Err(e) = client.post(&url).json(&payload).send().await {
            error!("❌ [TELEMETRY] Failed to send Telegram alert: {}", e);
        }
        
        // Small rate limit protection for Telegram API
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}