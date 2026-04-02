use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use csv::WriterBuilder;
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use tracing::info;

#[derive(Debug, Serialize)]
pub struct TradeRecord {
    pub trade_id: String,
    pub signal_key: String,
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
                "signal_key",
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

    pub fn has_signal_key(&self, signal_key: &str) -> Result<bool> {
        if !self.csv_path.exists() {
            return Ok(false);
        }

        let content = fs::read_to_string(&self.csv_path)?;
        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(true)
            .from_reader(content.as_bytes());

        let headers = rdr.headers()?.clone();
        let signal_key_col = headers
            .iter()
            .position(|h| h == "signal_key")
            .ok_or_else(|| anyhow!("colonne 'signal_key' introuvable dans le CSV"))?;

        for record in rdr.records() {
            let record = record?;
            if record.get(signal_key_col) == Some(signal_key) {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Met à jour le champ `outcome` d'un trade existant dans le CSV.
    /// Lit le fichier entier, modifie la ligne correspondante, réécrit via un fichier temporaire.
    pub fn update_outcome(&self, trade_id: &str, outcome: &str) -> Result<()> {
        let content = fs::read_to_string(&self.csv_path)?;
        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(true)
            .from_reader(content.as_bytes());

        let headers = rdr.headers()?.clone();
        let trade_id_col = headers.iter().position(|h| h == "trade_id").unwrap_or(0);
        let outcome_col = headers
            .iter()
            .position(|h| h == "outcome")
            .ok_or_else(|| anyhow!("colonne 'outcome' introuvable dans le CSV"))?;

        let records: Vec<Vec<String>> = rdr
            .records()
            .map(|r| r.map(|rec| rec.iter().map(|f| f.to_string()).collect()))
            .collect::<Result<_, _>>()?;

        let tmp_path = self.csv_path.with_extension("tmp");
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp_path)?;
        let mut wtr = WriterBuilder::new().has_headers(false).from_writer(file);
        wtr.write_record(&headers)?;

        for mut fields in records {
            if fields.get(trade_id_col).map(|v| v.as_str()) == Some(trade_id) {
                if let Some(f) = fields.get_mut(outcome_col) {
                    *f = outcome.to_string();
                }
            }
            wtr.write_record(&fields)?;
        }
        wtr.flush()?;
        drop(wtr);

        fs::rename(&tmp_path, &self.csv_path)?;
        info!("Outcome mis à jour | trade_id={} outcome={}", trade_id, outcome);
        Ok(())
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
    candle_high: f64,
    candle_low: f64,
    candle_open: f64,
    close: f64,
    color: &str,
    rsi: Option<f64>,
    series: Option<bool>,
    atr: Option<f64>,
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
    let atr_str = match atr {
        Some(a) => format!("{:.2}", a),
        None => "N/A".to_string(),
    };
    let range = candle_high - candle_low;
    let body_ratio_str = if range > 0.0 {
        format!("{:.0}%", (close - candle_open).abs() / range * 100.0)
    } else {
        "N/A".to_string()
    };
    let range_str = format!("{:.2}", range);
    info!(
        "[BOUGIE FERMÉE] {} {} | close={:.2} {} | RSI={} | série={} | ATR={} | range={} | body={} | {}",
        symbol,
        interval,
        close,
        color,
        rsi_str,
        series_str,
        atr_str,
        range_str,
        body_ratio_str,
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
