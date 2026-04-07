use chrono::{Duration, Utc};
use rusty_poly_bot::binance::Candle;
use rusty_poly_bot::config::{Config, ExecutionMode};
use rusty_poly_bot::logger::{TradeLogger, TradeRecord};
use rusty_poly_bot::polymarket::PolymarketClient;
use rusty_poly_bot::strategy::Prediction;
use rusty_poly_bot::tracker::{build_signal_key, PositionTracker};
use std::fs;
use std::sync::Arc;

fn tmp_dir(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusty_poly_bot_tracker_test_{}_{}",
        label,
        uuid::Uuid::new_v4()
    ))
}

fn make_config(logs_dir: &str) -> Config {
    Config {
        binance_ws_url: "wss://stream.binance.com:9443/ws".to_string(),
        symbol: "btcusdt".to_string(),
        interval: "5m".to_string(),
        execution_mode: ExecutionMode::DryRun,
        trade_amount_usdc: 1.0,
        polymarket_api_key: String::new(),
        polymarket_api_secret: String::new(),
        polymarket_api_url: "https://clob.polymarket.com".to_string(),
        logs_dir: logs_dir.to_string(),
        evm_private_key: None,
        polymarket_funder: None,
        polymarket_signature_type: None,
        strategy: "three_candle_rsi7_reversal".to_string(),
        rsi_overbought: 65.0,
        rsi_oversold: 35.0,
        polymarket_slug_prefix: "btc-updown-5m".to_string(),
    }
}

fn make_candle(close_time: chrono::DateTime<Utc>, open: f64, close: f64) -> Candle {
    Candle {
        open_time: close_time - Duration::minutes(5),
        close_time,
        open,
        high: open.max(close),
        low: open.min(close),
        close,
        volume: 1.0,
        is_closed: true,
    }
}

fn make_record(trade_id: &str, signal_key: &str, prediction: &str) -> TradeRecord {
    TradeRecord {
        trade_id: trade_id.to_string(),
        signal_key: signal_key.to_string(),
        symbol: "BTCUSDT".to_string(),
        interval: "5m".to_string(),
        signal_close_time_utc: "2024-01-01T00:00:00+00:00".to_string(),
        target_candle_open_time_utc: "2024-01-01T00:05:00+00:00".to_string(),
        prediction: prediction.to_string(),
        entry_side: "BUY".to_string(),
        entry_order_type: "MARKET".to_string(),
        order_status: "Matched".to_string(),
        signal_to_submit_start_ms: 10,
        submit_start_to_ack_ms: 5,
        signal_to_ack_ms: 15,
        trade_open_to_order_ack_ms: 20,
        outcome: "PENDING".to_string(),
    }
}

#[test]
fn test_build_signal_key_is_normalized() {
    let key = build_signal_key(" Three_Candle ", "BTC-UPDOWN-5M-123", &Prediction::Down);
    assert_eq!(key, "three_candle:btc-updown-5m-123:DOWN");
}

#[tokio::test]
async fn test_tracker_persists_pending_orders() {
    let dir = tmp_dir("persist");
    fs::create_dir_all(&dir).unwrap();
    let logger = Arc::new(TradeLogger::new(dir.to_str().unwrap()).unwrap());
    let client = Arc::new(PolymarketClient::new(make_config(dir.to_str().unwrap())));

    let tracker = PositionTracker::new(client.clone(), logger.clone(), dir.to_str().unwrap());
    tracker
        .track(
            "trade-1".to_string(),
            "order-1".to_string(),
            "signal-1".to_string(),
            Prediction::Up,
            Utc::now(),
            "MATCHED".to_string(),
        )
        .await;

    let reloaded = PositionTracker::new(client, logger, dir.to_str().unwrap());
    assert_eq!(reloaded.pending_count().await, 1);
    assert!(reloaded.is_signal_active("signal-1").await);

    fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn test_tracker_ignores_duplicate_signal_key() {
    let dir = tmp_dir("dedupe");
    fs::create_dir_all(&dir).unwrap();
    let logger = Arc::new(TradeLogger::new(dir.to_str().unwrap()).unwrap());
    let client = Arc::new(PolymarketClient::new(make_config(dir.to_str().unwrap())));

    let tracker = PositionTracker::new(client, logger, dir.to_str().unwrap());
    tracker
        .track(
            "trade-1".to_string(),
            "order-1".to_string(),
            "signal-1".to_string(),
            Prediction::Up,
            Utc::now(),
            "MATCHED".to_string(),
        )
        .await;
    tracker
        .track(
            "trade-2".to_string(),
            "order-2".to_string(),
            "signal-1".to_string(),
            Prediction::Down,
            Utc::now(),
            "MATCHED".to_string(),
        )
        .await;

    assert_eq!(tracker.pending_count().await, 1);
    fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn test_tracker_validates_win_with_green_candle_for_up() {
    let dir = tmp_dir("binance_win");
    fs::create_dir_all(&dir).unwrap();
    let logger = Arc::new(TradeLogger::new(dir.to_str().unwrap()).unwrap());
    let client = Arc::new(PolymarketClient::new(make_config(dir.to_str().unwrap())));

    let tracker = PositionTracker::new(client, logger.clone(), dir.to_str().unwrap());
    let target_close_time = Utc::now();
    logger
        .log_trade(&make_record("trade-1", "signal-1", "UP"))
        .unwrap();
    tracker
        .track(
            "trade-1".to_string(),
            "order-1".to_string(),
            "signal-1".to_string(),
            Prediction::Up,
            target_close_time,
            "MATCHED".to_string(),
        )
        .await;

    let candle = make_candle(target_close_time, 100.0, 110.0);
    tracker
        .validate_with_closed_candle(candle.close_time, candle.is_green())
        .await;

    let content = fs::read_to_string(dir.join("trades.csv")).unwrap();
    assert!(content.contains("WIN"));
    assert_eq!(tracker.pending_count().await, 0);
    fs::remove_dir_all(&dir).ok();
}
