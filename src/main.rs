use anyhow::Result;
use chrono::Utc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use uuid::Uuid;

use rusty_poly_bot::binance::{self, Candle};
use rusty_poly_bot::config::Config;
use rusty_poly_bot::logger::{
    log_candle_close, log_order_ack, log_order_sent, log_signal_detected, TradeLogger, TradeRecord,
};
use rusty_poly_bot::polymarket::PolymarketClient;
use rusty_poly_bot::strategies::three_candle_rsi7_reversal::ThreeCandleRsi7Reversal;
use rusty_poly_bot::strategy::{Prediction, Strategy};

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

    // Précharger 120 bougies historiques pour amorcer le RSI dès le démarrage
    match binance::fetch_historical_candles(&config.symbol, &config.interval, 120).await {
        Ok(candles) => {
            for candle in candles {
                active_strategy.on_closed_candle(&candle);
            }
        }
        Err(e) => {
            error!("Impossible de précharger l'historique Binance: {}", e);
        }
    }

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

        // P9 : slug sur le timestamp d'ouverture de la PROCHAINE bougie (close_time + 1ms)
        let next_open_ms = (candle.close_time + chrono::Duration::milliseconds(1))
            .timestamp_millis();
        let slug = PolymarketClient::build_slug(next_open_ms);

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

        // P8 : clamper les latences à 0 pour éviter des valeurs négatives (désync NTP)
        let signal_to_submit_start_ms = {
            let ms = (order_submit_started_at - signal_received_at).num_milliseconds();
            if ms < 0 {
                warn!("Latence signal→submit négative ({}ms) — désync NTP ?", ms);
            }
            ms.max(0)
        };
        let submit_start_to_ack_ms = {
            let ms = (order_result.ack_at - order_submit_started_at).num_milliseconds();
            if ms < 0 {
                warn!("Latence submit→ack négative ({}ms) — désync NTP ?", ms);
            }
            ms.max(0)
        };
        let signal_to_ack_ms = {
            let ms = (order_result.ack_at - signal_received_at).num_milliseconds();
            if ms < 0 {
                warn!("Latence signal→ack négative ({}ms) — désync NTP ?", ms);
            }
            ms.max(0)
        };
        let trade_open_to_order_ack_ms = {
            let ms = (order_result.ack_at - candle.close_time).num_milliseconds();
            if ms < 0 {
                warn!(
                    "Latence bougie→ack négative ({}ms) — désync horloge Binance/locale ?",
                    ms
                );
            }
            ms.max(0)
        };

        let token_id = match &signal.prediction {
            Prediction::Up => &market.up_token_id,
            Prediction::Down => &market.down_token_id,
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
            // P12 : as_str() pour cohérence avec order_status
            entry_order_type: config.execution_mode.as_str().to_string(),
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
