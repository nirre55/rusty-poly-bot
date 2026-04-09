use anyhow::Result;
use chrono::{Duration, Utc};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use uuid::Uuid;

use rusty_poly_bot::binance::{self, Candle};
use rusty_poly_bot::config::Config;
use rusty_poly_bot::logger::{
    log_candle_close, log_order_ack, log_order_sent, log_signal_detected, TradeLogger, TradeRecord,
};
use rusty_poly_bot::money::MoneyManager;
use rusty_poly_bot::polymarket::PolymarketClient;
use rusty_poly_bot::strategies::three_candle_rsi7_reversal::ThreeCandleRsi7Reversal;
use rusty_poly_bot::strategy::{Prediction, Strategy};
use rusty_poly_bot::tracker::{build_signal_key, PositionTracker};

fn parse_interval_duration(interval: &str) -> Result<Duration> {
    if interval.len() < 2 {
        anyhow::bail!("intervalle invalide: {}", interval);
    }
    let (value, unit) = interval.split_at(interval.len() - 1);
    let value: i64 = value.parse()?;
    match unit {
        "m" => Ok(Duration::minutes(value)),
        "h" => Ok(Duration::hours(value)),
        "d" => Ok(Duration::days(value)),
        _ => anyhow::bail!("unité d'intervalle non supportée: {}", interval),
    }
}

fn create_strategy(config: &Config) -> Result<Box<dyn Strategy>> {
    match config.strategy.as_str() {
        "three_candle_rsi7_reversal" => Ok(Box::new(ThreeCandleRsi7Reversal::new(
            config.rsi_overbought,
            config.rsi_oversold,
        ))),
        other => anyhow::bail!(
            "Stratégie '{}' inconnue. Stratégies disponibles: three_candle_rsi7_reversal",
            other
        ),
    }
}

/// Pré-fetch du marché suivant + warm caches SDK en arrière-plan.
fn spawn_prefetch_next_market(
    poly_client: &Arc<PolymarketClient>,
    close_time: chrono::DateTime<Utc>,
    interval_duration: Duration,
    slug_prefix: &str,
) {
    let poly = poly_client.clone();
    let future_open_ms =
        (close_time + interval_duration + chrono::Duration::milliseconds(1)).timestamp_millis();
    let future_slug = PolymarketClient::build_slug(slug_prefix, future_open_ms);
    tokio::spawn(async move {
        if let Ok(market) = poly.resolve_market(&future_slug).await {
            poly.warm_sdk_caches(&market).await;
        }
    });
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env()?;
    let interval_duration = parse_interval_duration(&config.interval)?;
    info!(
        "Démarrage rusty-poly-bot | mode={:?} symbol={} interval={} strategy={} rsi=[{},{}]",
        config.execution_mode, config.symbol, config.interval,
        config.strategy, config.rsi_oversold, config.rsi_overbought
    );

    let trade_logger = Arc::new(TradeLogger::new(&config.logs_dir)?);
    let poly_client = Arc::new(PolymarketClient::new(config.clone()));
    poly_client.warm_up().await;
    // Keep-alive : ping CLOB toutes les 20s pour garder la connexion TCP/TLS chaude
    tokio::spawn({
        let poly = poly_client.clone();
        async move { poly.run_keep_alive_loop().await }
    });
    let mut active_strategy = create_strategy(&config)?;

    // Money manager : Martingale progressive
    let money_manager = Arc::new(tokio::sync::Mutex::new(MoneyManager::new(
        config.trade_amount_usdc,
        config.martingale_multiplier,
        config.martingale_max_amount,
        &config.logs_dir,
    )));
    if config.martingale_multiplier > 1.0 {
        let mm = money_manager.lock().await;
        info!(
            "Martingale activée | base={:.2} USDC multiplier={:.2} montant_courant={:.2} USDC (losses={})",
            config.trade_amount_usdc, config.martingale_multiplier,
            mm.current_amount(), mm.consecutive_losses()
        );
    }

    // Tracker V3 : suit les ordres ouverts et met à jour outcome dans le CSV
    let tracker = Arc::new(PositionTracker::new(
        poly_client.clone(),
        trade_logger.clone(),
        money_manager.clone(),
        &config.logs_dir,
    ));
    tokio::spawn({
        let tracker = tracker.clone();
        async move { tracker.run_poll_loop().await }
    });

    // Précharger 120 bougies historiques pour amorcer le RSI dès le démarrage.
    // On exclut la dernière bougie si elle est encore ouverte (close_time >= now),
    // identique au filtre build_closed_candles() du script Python de référence.
    match binance::fetch_historical_candles(&config.symbol, &config.interval, 120).await {
        Ok(candles) => {
            let now_ms = Utc::now().timestamp_millis();
            let closed: Vec<_> = candles
                .into_iter()
                .filter(|c| c.close_time.timestamp_millis() < now_ms)
                .collect();
            info!(
                "Préchargement : {} bougies fermées utilisées pour le warmup RSI",
                closed.len()
            );
            for candle in closed {
                active_strategy.warmup(&candle);
            }
        }
        Err(e) => {
            error!("Impossible de précharger l'historique Binance: {}", e);
        }
    }

    // Boucle de résilience : relance le stream Binance si le channel se ferme
    loop {
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

        let signal = active_strategy.on_closed_candle(&candle);

        let color = if candle.is_green() { "VERT" } else { "ROUGE" };
        log_candle_close(
            &config.symbol,
            &config.interval,
            candle.high,
            candle.low,
            candle.open,
            candle.close,
            color,
            active_strategy.current_rsi(),
            active_strategy.current_series(),
            active_strategy.current_atr(),
            &candle.close_time,
        );

        // Slug du marché COURANT (pré-fetché par la bougie précédente → cache hit)
        let next_open_ms = (candle.close_time + chrono::Duration::milliseconds(1))
            .timestamp_millis();
        let slug = PolymarketClient::build_slug(&config.polymarket_slug_prefix, next_open_ms);

        let Some(signal) = signal else {
            tracker
                .validate_with_closed_candle(candle.close_time, candle.is_green())
                .await;
            spawn_prefetch_next_market(&poly_client, candle.close_time, interval_duration, &config.polymarket_slug_prefix);
            continue;
        };

        log_signal_detected(
            &signal.strategy_name,
            &signal.prediction.to_string(),
            signal.rsi,
        );

        let target_close_time = candle.close_time + interval_duration;
        let signal_key = build_signal_key(&signal.strategy_name, &slug, &signal.prediction);

        if tracker.is_signal_active(&signal_key).await {
            warn!(
                "Signal déjà en cours de suivi — ordre ignoré | signal_key={}",
                signal_key
            );
            tracker
                .validate_with_closed_candle(candle.close_time, candle.is_green())
                .await;
            continue;
        }
        match trade_logger.has_signal_key(&signal_key) {
            Ok(true) => {
                warn!(
                    "Signal déjà exécuté précédemment — ordre ignoré | signal_key={}",
                    signal_key
                );
                tracker
                    .validate_with_closed_candle(candle.close_time, candle.is_green())
                    .await;
                continue;
            }
            Ok(false) => {}
            Err(e) => {
                warn!(
                    "Impossible de vérifier l'historique des signaux ({}), poursuite prudente",
                    e
                );
            }
        }

        // Marché pré-fetché par la bougie précédente → cache hit (~0ms).
        // Premier signal après restart → fetch normal (~100ms).
        let market = match poly_client.resolve_market(&slug).await {
            Ok(m) => m,
            Err(e) => {
                error!("Impossible de résoudre le marché Polymarket: {}", e);
                tracker
                    .validate_with_closed_candle(candle.close_time, candle.is_green())
                    .await;
                continue;
            }
        };

        let trade_amount = money_manager.lock().await.current_amount();
        let order_submit_started_at = Utc::now();

        let order_result = match poly_client.place_order(&signal, &market, trade_amount).await {
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
            if ms < -2_000 {
                warn!(
                    "Latence bougie→ack très négative ({}ms) — désync horloge Binance/locale ?",
                    ms
                );
            }
            ms.max(0)
        };

        let token_id = match &signal.prediction {
            Prediction::Up => &market.up_token_id,
            Prediction::Down => &market.down_token_id,
        };

        log_order_sent(&order_result.order_id, token_id, trade_amount);
        log_order_ack(&order_result.order_id, &order_result.status, signal_to_ack_ms);

        let trade_id = Uuid::new_v4().to_string();

        let record = TradeRecord {
            trade_id: trade_id.clone(),
            signal_key: signal_key.clone(),
            symbol: config.symbol.clone(),
            interval: config.interval.clone(),
            signal_close_time_utc: signal.signal_candle_close_time.to_rfc3339(),
            target_candle_open_time_utc: candle.close_time.to_rfc3339(),
            prediction: signal.prediction.to_string(),
            entry_side: "BUY".to_string(),
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

        // V3 : enregistrer l'ordre pour suivi (no-op pour les ordres dry-run)
        tracker
            .track(
                trade_id,
                order_result.order_id,
                signal_key,
                signal.prediction.clone(),
                target_close_time,
                order_result.status.clone(),
            )
            .await;

        // Validation tracker + pré-fetch du marché suivant après l'ordre
        tracker
            .validate_with_closed_candle(candle.close_time, candle.is_green())
            .await;
        spawn_prefetch_next_market(&poly_client, candle.close_time, interval_duration, &config.polymarket_slug_prefix);
    }

    // Le channel s'est fermé (WS task morte) — relancer
    warn!("[RECONNECT] Channel Binance fermé — relance du stream dans 5s…");
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    // Ré-authentifier le client SDK Polymarket au cas où
    poly_client.warm_up().await;

    } // fin boucle de résilience
}
