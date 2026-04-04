use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use csv::StringRecord;
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

        Self::migrate_csv_if_needed(&csv_path)?;

        Ok(Self { csv_path })
    }

    pub fn has_signal_key(&self, signal_key: &str) -> Result<bool> {
        if !self.csv_path.exists() {
            return Ok(false);
        }

        let content = fs::read_to_string(&self.csv_path)?;
        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(true)
            .flexible(true)
            .from_reader(content.as_bytes());

        let headers = rdr.headers()?.clone();
        let Some(signal_key_col) = headers.iter().position(|h| h == "signal_key") else {
            return Ok(false);
        };

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
        self.update_trade_field(trade_id, "outcome", outcome)
    }

    pub fn update_order_status(&self, trade_id: &str, order_status: &str) -> Result<()> {
        self.update_trade_field(trade_id, "order_status", order_status)
    }

    fn update_trade_field(&self, trade_id: &str, column_name: &str, new_value: &str) -> Result<()> {
        let content = fs::read_to_string(&self.csv_path)?;
        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(true)
            .flexible(true)
            .from_reader(content.as_bytes());

        let headers = rdr.headers()?.clone();
        let trade_id_col = headers.iter().position(|h| h == "trade_id").unwrap_or(0);
        let target_col = headers
            .iter()
            .position(|h| h == column_name)
            .ok_or_else(|| anyhow!("colonne '{}' introuvable dans le CSV", column_name))?;

        let records: Vec<Vec<String>> = rdr
            .records()
            .map(|r| {
                r.map(|rec| {
                    let mut fields: Vec<String> = rec.iter().map(|f| f.to_string()).collect();
                    while fields.len() < headers.len() {
                        fields.push(String::new());
                    }
                    if fields.len() > headers.len() {
                        fields.truncate(headers.len());
                    }
                    fields
                })
            })
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
                if let Some(f) = fields.get_mut(target_col) {
                    *f = new_value.to_string();
                }
            }
            wtr.write_record(&fields)?;
        }
        wtr.flush()?;
        drop(wtr);

        fs::rename(&tmp_path, &self.csv_path)?;
        info!(
            "Trade mis à jour | trade_id={} {}={}",
            trade_id, column_name, new_value
        );
        Ok(())
    }

    fn migrate_csv_if_needed(csv_path: &PathBuf) -> Result<()> {
        if !csv_path.exists() {
            return Ok(());
        }

        let content = fs::read_to_string(csv_path)?;
        if content.trim().is_empty() {
            return Ok(());
        }

        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(true)
            .flexible(true)
            .from_reader(content.as_bytes());
        let headers = rdr.headers()?.clone();

        if headers.iter().any(|h| h == "signal_key") {
            return Ok(());
        }

        let old_header_len = headers.len();
        let new_headers = Self::csv_headers();
        let new_header_len = new_headers.len();

        let mut migrated_rows = Vec::new();
        for record in rdr.records() {
            let record = record?;
            let migrated = Self::migrate_record_to_current_schema(&record, old_header_len, new_header_len);
            migrated_rows.push(migrated);
        }

        let tmp_path = csv_path.with_extension("tmp");
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp_path)?;
        let mut wtr = WriterBuilder::new().has_headers(false).from_writer(file);
        wtr.write_record(&new_headers)?;
        for row in migrated_rows {
            wtr.write_record(row)?;
        }
        wtr.flush()?;
        drop(wtr);

        fs::rename(&tmp_path, csv_path)?;
        info!("Migration CSV effectuée | fichier={} -> schéma avec signal_key", csv_path.display());
        Ok(())
    }

    fn migrate_record_to_current_schema(
        record: &StringRecord,
        old_header_len: usize,
        new_header_len: usize,
    ) -> Vec<String> {
        let mut fields: Vec<String> = record.iter().map(|f| f.to_string()).collect();

        if fields.len() == old_header_len {
            fields.insert(1, String::new());
        }

        while fields.len() < new_header_len {
            fields.push(String::new());
        }
        if fields.len() > new_header_len {
            fields.truncate(new_header_len);
        }

        fields
    }

    fn csv_headers() -> [&'static str; 15] {
        [
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
        ]
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
