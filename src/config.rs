use anyhow::Result;
use std::env;
use tracing::warn;

#[derive(Debug, Clone)]
pub enum ExecutionMode {
    DryRun,
    Market,
    Limit,
}

impl ExecutionMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExecutionMode::DryRun => "DRY_RUN",
            ExecutionMode::Market => "MARKET",
            ExecutionMode::Limit => "LIMIT",
        }
    }
}

// P13 : impl Debug manuel pour masquer les secrets dans les logs
#[derive(Clone)]
pub struct Config {
    pub binance_ws_url: String,
    pub symbol: String,
    pub interval: String,
    pub execution_mode: ExecutionMode,
    pub trade_amount_usdc: f64,
    #[allow(dead_code)]
    pub polymarket_api_key: String,
    #[allow(dead_code)]
    pub polymarket_api_secret: String,
    #[allow(dead_code)]
    pub polymarket_api_url: String,
    pub logs_dir: String,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("binance_ws_url", &self.binance_ws_url)
            .field("symbol", &self.symbol)
            .field("interval", &self.interval)
            .field("execution_mode", &self.execution_mode)
            .field("trade_amount_usdc", &self.trade_amount_usdc)
            .field("polymarket_api_key", &"[REDACTED]")
            .field("polymarket_api_secret", &"[REDACTED]")
            .field("polymarket_api_url", &self.polymarket_api_url)
            .field("logs_dir", &self.logs_dir)
            .finish()
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        let mode = env::var("EXECUTION_MODE").unwrap_or_else(|_| "dry-run".to_string());
        // P10 : avertir si la valeur est inconnue au lieu de fallback silencieux
        let execution_mode = match mode.as_str() {
            "market" => ExecutionMode::Market,
            "limit" => ExecutionMode::Limit,
            "dry-run" | "dryrun" => ExecutionMode::DryRun,
            _ => {
                warn!(
                    "EXECUTION_MODE '{}' non reconnu — mode dry-run utilisé par défaut",
                    mode
                );
                ExecutionMode::DryRun
            }
        };

        // P11 : valider que TRADE_AMOUNT_USDC est un nombre strictement positif
        let raw_amount = env::var("TRADE_AMOUNT_USDC").unwrap_or_else(|_| "10.0".to_string());
        let trade_amount_usdc = match raw_amount.parse::<f64>() {
            Ok(v) if v > 0.0 => v,
            Ok(v) => {
                warn!(
                    "TRADE_AMOUNT_USDC={} invalide (doit être > 0) — valeur par défaut 10.0 USDC utilisée",
                    v
                );
                10.0
            }
            Err(_) => {
                warn!(
                    "TRADE_AMOUNT_USDC='{}' non parseable — valeur par défaut 10.0 USDC utilisée",
                    raw_amount
                );
                10.0
            }
        };

        Ok(Config {
            binance_ws_url: env::var("BINANCE_WS_URL")
                .unwrap_or_else(|_| "wss://stream.binance.com:9443/ws".to_string()),
            symbol: env::var("SYMBOL").unwrap_or_else(|_| "btcusdt".to_string()),
            interval: env::var("INTERVAL").unwrap_or_else(|_| "5m".to_string()),
            execution_mode,
            trade_amount_usdc,
            polymarket_api_key: env::var("POLYMARKET_API_KEY").unwrap_or_default(),
            polymarket_api_secret: env::var("POLYMARKET_API_SECRET").unwrap_or_default(),
            polymarket_api_url: env::var("POLYMARKET_API_URL")
                .unwrap_or_else(|_| "https://clob.polymarket.com".to_string()),
            logs_dir: env::var("LOGS_DIR").unwrap_or_else(|_| "logs".to_string()),
        })
    }
}
