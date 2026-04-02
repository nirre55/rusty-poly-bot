use rusty_poly_bot::config::{Config, ExecutionMode};
use rusty_poly_bot::logger::TradeLogger;
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
        )
        .await;
    tracker
        .track(
            "trade-2".to_string(),
            "order-2".to_string(),
            "signal-1".to_string(),
        )
        .await;

    assert_eq!(tracker.pending_count().await, 1);
    fs::remove_dir_all(&dir).ok();
}
