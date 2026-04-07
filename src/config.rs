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
    /// Clé privée EVM (hex, avec ou sans "0x"). Requise pour ExecutionMode::Market.
    pub evm_private_key: Option<String>,
    /// Adresse funder Polymarket (proxy/safe) si différente de l'EOA signataire.
    pub polymarket_funder: Option<String>,
    /// Signature type Polymarket: 0=EOA, 1=POLY_PROXY, 2=GNOSIS_SAFE.
    pub polymarket_signature_type: Option<u8>,
    /// Nom de la stratégie à utiliser (ex: "three_candle_rsi7_reversal").
    pub strategy: String,
    /// Seuil RSI haut (suracheté) — signal DOWN si RSI >= ce seuil. Défaut: 65.0
    pub rsi_overbought: f64,
    /// Seuil RSI bas (survendu) — signal UP si RSI <= ce seuil. Défaut: 35.0
    pub rsi_oversold: f64,
    /// Préfixe slug Polymarket (ex: "btc-updown-5m"). Format final: {prefix}-{timestamp}
    pub polymarket_slug_prefix: String,
    /// Multiplicateur Martingale après chaque loss. 1.0 = désactivé. Défaut: 1.0
    pub martingale_multiplier: f64,
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
            .field("evm_private_key", &"[REDACTED]")
            .field("polymarket_funder", &self.polymarket_funder)
            .field("polymarket_signature_type", &self.polymarket_signature_type)
            .field("strategy", &self.strategy)
            .field("rsi_overbought", &self.rsi_overbought)
            .field("rsi_oversold", &self.rsi_oversold)
            .field("polymarket_slug_prefix", &self.polymarket_slug_prefix)
            .field("martingale_multiplier", &self.martingale_multiplier)
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

        let polymarket_signature_type = match env::var("POLYMARKET_SIGNATURE_TYPE") {
            Ok(raw) => match raw.parse::<u8>() {
                Ok(v @ 0..=2) => Some(v),
                Ok(v) => {
                    warn!(
                        "POLYMARKET_SIGNATURE_TYPE={} invalide (attendu 0, 1 ou 2) — valeur ignorée",
                        v
                    );
                    None
                }
                Err(_) => {
                    warn!(
                        "POLYMARKET_SIGNATURE_TYPE='{}' non parseable — valeur ignorée",
                        raw
                    );
                    None
                }
            },
            Err(_) => None,
        };

        let martingale_multiplier = env::var("MARTINGALE_MULTIPLIER")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(1.0);

        let rsi_overbought = env::var("RSI_OVERBOUGHT")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(65.0);
        let rsi_oversold = env::var("RSI_OVERSOLD")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(35.0);

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
            evm_private_key: env::var("POLYMARKET_PRIVATE_KEY").ok(),
            polymarket_funder: env::var("POLYMARKET_FUNDER").ok(),
            polymarket_signature_type,
            strategy: env::var("STRATEGY")
                .unwrap_or_else(|_| "three_candle_rsi7_reversal".to_string()),
            rsi_overbought,
            rsi_oversold,
            polymarket_slug_prefix: env::var("POLYMARKET_SLUG_PREFIX")
                .unwrap_or_else(|_| "btc-updown-5m".to_string()),
            martingale_multiplier,
        })
    }
}
