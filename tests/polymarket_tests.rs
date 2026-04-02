use chrono::Utc;
use rusty_poly_bot::config::{Config, ExecutionMode};
use rusty_poly_bot::polymarket::{MarketInfo, PolymarketClient};
use rusty_poly_bot::strategy::{Prediction, Signal};

fn make_config(mode: ExecutionMode) -> Config {
    Config {
        binance_ws_url: "wss://stream.binance.com:9443/ws".to_string(),
        symbol: "btcusdt".to_string(),
        interval: "5m".to_string(),
        execution_mode: mode,
        trade_amount_usdc: 10.0,
        polymarket_api_key: String::new(),
        polymarket_api_secret: String::new(),
        polymarket_api_url: "https://clob.polymarket.com".to_string(),
        logs_dir: "logs".to_string(),
        evm_private_key: None,
        polymarket_funder: None,
        polymarket_signature_type: None,
    }
}

fn make_signal(prediction: Prediction) -> Signal {
    Signal {
        prediction,
        signal_candle_close_time: Utc::now(),
        rsi: 72.0,
        strategy_name: "test".to_string(),
    }
}

fn make_market() -> MarketInfo {
    MarketInfo {
        condition_id: "cond_123".to_string(),
        up_token_id: "up_token".to_string(),
        down_token_id: "down_token".to_string(),
        slug: "btc-updown-5m-20240309".to_string(),
        order_min_size: 5.0,
    }
}

// --- build_slug ---

#[test]
fn test_build_slug_known_timestamp() {
    // 1710000000000 ms → Unix 1710000000 s
    let slug = PolymarketClient::build_slug(1710000000000);
    assert_eq!(slug, "btc-updown-5m-1710000000");
}

#[test]
fn test_build_slug_format_prefix() {
    let slug = PolymarketClient::build_slug(1710000000000);
    assert!(slug.starts_with("btc-updown-5m-"));
}

#[test]
fn test_build_slug_suffix_is_unix_seconds() {
    // Le suffixe est le timestamp en secondes (pas YYYYMMDD)
    let slug = PolymarketClient::build_slug(1710000000000);
    let suffix = slug.strip_prefix("btc-updown-5m-").unwrap();
    assert_eq!(suffix, "1710000000");
    assert!(suffix.chars().all(|c| c.is_ascii_digit()));
}

#[test]
fn test_build_slug_ms_to_seconds_truncation() {
    // 1704067200000 ms → 1704067200 s
    let slug = PolymarketClient::build_slug(1704067200000);
    assert_eq!(slug, "btc-updown-5m-1704067200");
}

#[test]
fn test_build_slug_different_candles_produce_different_slugs() {
    // Deux bougies 5m consécutives (300 000 ms d'écart)
    let slug1 = PolymarketClient::build_slug(1710000000000);
    let slug2 = PolymarketClient::build_slug(1710000300000);
    assert_ne!(slug1, slug2);
}

// --- place_order ---

#[tokio::test]
async fn test_place_order_dryrun_returns_ok() {
    let client = PolymarketClient::new(make_config(ExecutionMode::DryRun));
    let signal = make_signal(Prediction::Up);
    let market = make_market();

    let result = client.place_order(&signal, &market).await;
    assert!(result.is_ok(), "DryRun doit retourner Ok");

    let order = result.unwrap();
    assert_eq!(order.status, "DRY_RUN");
    assert!(order.order_id.starts_with("dry-run-"));
}

#[tokio::test]
async fn test_place_order_dryrun_down_signal() {
    let client = PolymarketClient::new(make_config(ExecutionMode::DryRun));
    let signal = make_signal(Prediction::Down);
    let market = make_market();

    let result = client.place_order(&signal, &market).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().status, "DRY_RUN");
}

/// P3 : en mode Market, place_order doit retourner Err (non implémenté en V2)
#[tokio::test]
async fn test_place_order_market_mode_returns_err() {
    let client = PolymarketClient::new(make_config(ExecutionMode::Market));
    let signal = make_signal(Prediction::Up);
    let market = make_market();

    let result = client.place_order(&signal, &market).await;
    assert!(result.is_err(), "Mode Market non implémenté doit retourner Err");
}

/// P3 : en mode Limit, place_order doit retourner Err (non implémenté en V2)
#[tokio::test]
async fn test_place_order_limit_mode_returns_err() {
    let client = PolymarketClient::new(make_config(ExecutionMode::Limit));
    let signal = make_signal(Prediction::Down);
    let market = make_market();

    let result = client.place_order(&signal, &market).await;
    assert!(result.is_err(), "Mode Limit non implémenté doit retourner Err");
}

/// Vérifie que ack_at >= submitted_at (pas de latence négative dans le dry-run)
#[tokio::test]
async fn test_place_order_dryrun_timestamps_ordered() {
    let client = PolymarketClient::new(make_config(ExecutionMode::DryRun));
    let signal = make_signal(Prediction::Up);
    let market = make_market();

    let before = Utc::now();
    let order = client.place_order(&signal, &market).await.unwrap();
    assert!(order.ack_at >= before, "ack_at doit être >= au timestamp avant l'appel");
    assert!(order.ack_at >= order.submitted_at, "ack_at doit être >= submitted_at");
}
