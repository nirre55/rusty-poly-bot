use anyhow::Result;
use chrono::{DateTime, Utc};
use csv::WriterBuilder;
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use tracing::info;

#[derive(Debug, Serialize)]
pub struct TradeRecord {
    pub trade_id: String,
    pub symbol: String,
    pub interval: String,
    pub signal_close_time_utc: String,
    pub target_candle_open_time_utc: String,
    pub prediction: String,
    pub entry_side: String,
    pub entry_order_type: String,
    pub order_status: String,
    pub signal_to_submit_start_ms: i64,
    pub submit_start_to_ack_ms: i64,
    pub signal_to_ack_ms: i64,
    pub trade_open_to_order_ack_ms: i64,
    pub outcome: String,
}

pub struct TradeLogger {
    csv_path: PathBuf,
}

impl TradeLogger {
    pub fn new(logs_dir: &str) -> Result<Self> {
        fs::create_dir_all(logs_dir)?;
        let csv_path = PathBuf::from(logs_dir).join("trades.csv");

        // P7 : écrire les headers si le fichier n'existe pas OU s'il est vide
        // (couvre le cas d'un crash pendant l'initialisation qui laisse un fichier vide)
        let needs_header = !csv_path.exists()
            || fs::metadata(&csv_path)
                .map(|m| m.len() == 0)
                .unwrap_or(true);

        if needs_header {
            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&csv_path)?;
            let mut wtr = WriterBuilder::new().has_headers(true).from_writer(file);
            wtr.write_record(&[
                "trade_id",
                "symbol",
                "interval",
                "signal_close_time_utc",
                "target_candle_open_time_utc",
                "prediction",
                "entry_side",
                "entry_order_type",
                "order_status",
                "signal_to_submit_start_ms",
                "submit_start_to_ack_ms",
                "signal_to_ack_ms",
                "trade_open_to_order_ack_ms",
                "outcome",
            ])?;
            wtr.flush()?;
        }

        Ok(Self { csv_path })
    }

    pub fn log_trade(&self, record: &TradeRecord) -> Result<()> {
        let file = OpenOptions::new().append(true).open(&self.csv_path)?;
        let mut wtr = WriterBuilder::new().has_headers(false).from_writer(file);
        wtr.serialize(record)?;
        wtr.flush()?;
        info!(
            "Trade enregistré | id={} prediction={} status={}",
            record.trade_id, record.prediction, record.order_status
        );
        Ok(())
    }
}

// --- Fonctions de log console ---

pub fn log_candle_close(
    symbol: &str,
    interval: &str,
    close: f64,
    color: &str,
    rsi: Option<f64>,
    series: Option<bool>,
    close_time: &DateTime<Utc>,
) {
    let rsi_str = match rsi {
        Some(r) => format!("{:.2}", r),
        None => "N/A".to_string(),
    };
    let series_str = match series {
        Some(true) => "3xVERT",
        Some(false) => "3xROUGE",
        None => "mixte",
    };
    info!(
        "[BOUGIE FERMÉE] {} {} | close={:.2} {} | RSI={} | série={} | {}",
        symbol,
        interval,
        close,
        color,
        rsi_str,
        series_str,
        close_time.format("%Y-%m-%d %H:%M:%S UTC")
    );
}

pub fn log_signal_detected(strategy: &str, prediction: &str, rsi: f64) {
    info!(
        "[SIGNAL] strategy={} prediction={} rsi={:.2}",
        strategy, prediction, rsi
    );
}

pub fn log_order_sent(order_id: &str, token_id: &str, amount: f64) {
    info!(
        "[ORDRE ENVOYÉ] id={} token={} amount={} USDC",
        order_id, token_id, amount
    );
}

pub fn log_order_ack(order_id: &str, status: &str, latency_ms: i64) {
    info!(
        "[ORDRE ACK] id={} status={} latence={}ms",
        order_id, status, latency_ms
    );
}
