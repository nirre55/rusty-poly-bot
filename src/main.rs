mod binance;
mod config;
mod logger;
mod polymarket;
mod strategies;
mod strategy;

use anyhow::Result;
use chrono::Utc;
use tokio::sync::mpsc;
use tracing::{error, info};
use uuid::Uuid;

use binance::Candle;
use config::Config;
use logger::{
    log_candle_close, log_order_ack, log_order_sent, log_signal_detected, TradeLogger, TradeRecord,
};
use polymarket::PolymarketClient;
use strategies::three_candle_rsi7_reversal::ThreeCandleRsi7Reversal;
use strategy::Strategy;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env()?;
    info!(
        "Démarrage rusty-poly-bot | mode={:?} symbol={} interval={}",
        config.execution_mode, config.symbol, config.interval
    );

    let trade_logger = TradeLogger::new(&config.logs_dir)?;
    let poly_client = PolymarketClient::new(config.clone());
    let mut active_strategy: Box<dyn Strategy> = Box::new(ThreeCandleRsi7Reversal::new());

    let (tx, mut rx) = mpsc::channel::<Candle>(64);

    // Lancer le stream Binance dans une tâche dédiée
    let ws_url = config.binance_ws_url.clone();
    let symbol = config.symbol.clone();
    let interval = config.interval.clone();

    tokio::spawn(async move {
        if let Err(e) = binance::stream_candles(&ws_url, &symbol, &interval, tx).await {
            error!("Erreur stream Binance: {}", e);
        }
    });

    // Boucle principale : traiter les bougies fermées
    while let Some(candle) = rx.recv().await {
        let signal_received_at = Utc::now();

        log_candle_close(
            &config.symbol,
            &config.interval,
            candle.close,
            &candle.close_time,
        );

        let Some(signal) = active_strategy.on_closed_candle(&candle) else {
            continue;
        };

        log_signal_detected(
            &signal.strategy_name,
            &signal.prediction.to_string(),
            signal.rsi,
        );

        let slug = PolymarketClient::build_slug(candle.open_time.timestamp_millis());

        let market = match poly_client.resolve_market(&slug).await {
            Ok(m) => m,
            Err(e) => {
                error!("Impossible de résoudre le marché Polymarket: {}", e);
                continue;
            }
        };

        let order_submit_started_at = Utc::now();

        let order_result = match poly_client.place_order(&signal, &market).await {
            Ok(r) => r,
            Err(e) => {
                error!("Erreur lors de l'envoi de l'ordre: {}", e);
                continue;
            }
        };

        // Mesures de latence
        let signal_to_submit_start_ms =
            (order_submit_started_at - signal_received_at).num_milliseconds();
        let submit_start_to_ack_ms =
            (order_result.ack_at - order_submit_started_at).num_milliseconds();
        let signal_to_ack_ms = (order_result.ack_at - signal_received_at).num_milliseconds();
        let trade_open_to_order_ack_ms =
            (order_result.ack_at - candle.close_time).num_milliseconds();

        let token_id = match &signal.prediction {
            strategy::Prediction::Up => &market.up_token_id,
            strategy::Prediction::Down => &market.down_token_id,
        };

        log_order_sent(&order_result.order_id, token_id, config.trade_amount_usdc);
        log_order_ack(&order_result.order_id, &order_result.status, signal_to_ack_ms);

        let record = TradeRecord {
            trade_id: Uuid::new_v4().to_string(),
            symbol: config.symbol.clone(),
            interval: config.interval.clone(),
            signal_close_time_utc: signal.signal_candle_close_time.to_rfc3339(),
            target_candle_open_time_utc: candle.close_time.to_rfc3339(),
            prediction: signal.prediction.to_string(),
            entry_side: "BUY".to_string(),
            entry_order_type: format!("{:?}", config.execution_mode),
            order_status: order_result.status.clone(),
            signal_to_submit_start_ms,
            submit_start_to_ack_ms,
            signal_to_ack_ms,
            trade_open_to_order_ack_ms,
            outcome: "PENDING".to_string(),
        };

        if let Err(e) = trade_logger.log_trade(&record) {
            error!("Erreur lors de l'enregistrement du trade: {}", e);
        }
    }

    Ok(())
}
