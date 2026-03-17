use anyhow::Result;
use std::env;

#[derive(Debug, Clone)]
pub enum ExecutionMode {
    DryRun,
    Market,
    Limit,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub binance_ws_url: String,
    pub symbol: String,
    pub interval: String,
    pub execution_mode: ExecutionMode,
    pub trade_amount_usdc: f64,
    // Utilisés en V2 (ordres réels Polymarket)
    #[allow(dead_code)]
    pub polymarket_api_key: String,
    #[allow(dead_code)]
    pub polymarket_api_secret: String,
    #[allow(dead_code)]
    pub polymarket_api_url: String,
    pub logs_dir: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        let mode = env::var("EXECUTION_MODE").unwrap_or_else(|_| "dry-run".to_string());
        let execution_mode = match mode.as_str() {
            "market" => ExecutionMode::Market,
            "limit" => ExecutionMode::Limit,
            _ => ExecutionMode::DryRun,
        };

        Ok(Config {
            binance_ws_url: env::var("BINANCE_WS_URL")
                .unwrap_or_else(|_| "wss://stream.binance.com:9443/ws".to_string()),
            symbol: env::var("SYMBOL").unwrap_or_else(|_| "btcusdt".to_string()),
            interval: env::var("INTERVAL").unwrap_or_else(|_| "5m".to_string()),
            execution_mode,
            trade_amount_usdc: env::var("TRADE_AMOUNT_USDC")
                .unwrap_or_else(|_| "10.0".to_string())
                .parse()
                .unwrap_or(10.0),
            polymarket_api_key: env::var("POLYMARKET_API_KEY").unwrap_or_default(),
            polymarket_api_secret: env::var("POLYMARKET_API_SECRET").unwrap_or_default(),
            polymarket_api_url: env::var("POLYMARKET_API_URL")
                .unwrap_or_else(|_| "https://clob.polymarket.com".to_string()),
            logs_dir: env::var("LOGS_DIR").unwrap_or_else(|_| "logs".to_string()),
        })
    }
}
