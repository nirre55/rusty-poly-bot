use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use serde::Deserialize;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

// Les champs high, low, volume seront utilisés par les stratégies futures (EMA, ATR, etc.)
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Candle {
    pub open_time: DateTime<Utc>,
    pub close_time: DateTime<Utc>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    #[allow(dead_code)]
    pub is_closed: bool,
}

impl Candle {
    pub fn is_green(&self) -> bool {
        self.close >= self.open
    }

    pub fn is_red(&self) -> bool {
        self.close < self.open
    }
}

#[derive(Debug, Deserialize)]
struct KlineEvent {
    #[serde(rename = "k")]
    kline: KlineData,
}

#[derive(Debug, Deserialize)]
struct KlineData {
    #[serde(rename = "t")]
    open_time: i64,
    #[serde(rename = "T")]
    close_time: i64,
    #[serde(rename = "o")]
    open: String,
    #[serde(rename = "h")]
    high: String,
    #[serde(rename = "l")]
    low: String,
    #[serde(rename = "c")]
    close: String,
    #[serde(rename = "v")]
    volume: String,
    #[serde(rename = "x")]
    is_closed: bool,
}

/// Parse une réponse brute de l'API klines Binance en Vec<Candle>.
/// Les entrées malformées sont ignorées (filter_map retourne None).
pub fn parse_klines(rows: Vec<serde_json::Value>) -> Vec<Candle> {
    rows.into_iter()
        .filter_map(|row| {
            let arr = row.as_array()?;
            if arr.len() < 7 {
                return None;
            }
            // P2 : ? au lieu de unwrap_or_default() — timestamp invalide → bougie ignorée
            Some(Candle {
                open_time: DateTime::from_timestamp_millis(arr[0].as_i64()?)?,
                close_time: DateTime::from_timestamp_millis(arr[6].as_i64()?)?,
                open: arr[1].as_str()?.parse().ok()?,
                high: arr[2].as_str()?.parse().ok()?,
                low: arr[3].as_str()?.parse().ok()?,
                close: arr[4].as_str()?.parse().ok()?,
                volume: arr[5].as_str()?.parse().ok()?,
                is_closed: true,
            })
        })
        .collect()
}

/// Récupère les `limit` dernières bougies fermées via l'API REST Binance.
/// Utilisé au démarrage pour précharger l'historique avant le WebSocket.
pub async fn fetch_historical_candles(
    symbol: &str,
    interval: &str,
    limit: u32,
) -> Result<Vec<Candle>> {
    let url = format!(
        "https://api.binance.com/api/v3/klines?symbol={}&interval={}&limit={}",
        symbol.to_uppercase(),
        interval,
        limit
    );

    // P6 : client avec timeout pour éviter un blocage indéfini
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("Erreur création client HTTP Binance")?;

    let response = client
        .get(&url)
        .send()
        .await
        .context("Erreur HTTP Binance REST")?
        .json::<Vec<serde_json::Value>>()
        .await
        .context("Erreur parsing JSON klines")?;

    let candles = parse_klines(response);

    info!(
        "Préchargement : {} bougies historiques chargées ({} {})",
        candles.len(),
        symbol,
        interval
    );
    Ok(candles)
}

pub async fn stream_candles(
    url: &str,
    symbol: &str,
    interval: &str,
    tx: mpsc::Sender<Candle>,
) -> Result<()> {
    let ws_url = format!("{}/{symbol}@kline_{interval}", url);
    info!("Connecting to Binance WebSocket: {}", ws_url);

    loop {
        // P6 : timeout sur la tentative de connexion WebSocket
        let connect_result =
            tokio::time::timeout(Duration::from_secs(15), connect_async(&ws_url)).await;

        match connect_result {
            Ok(Ok((ws_stream, _))) => {
                info!("Connected to Binance WebSocket");
                let (_, mut read) = ws_stream.split();

                while let Some(msg) = read.next().await {
                    match msg {
                        Ok(Message::Text(text)) => {
                            match serde_json::from_str::<KlineEvent>(&text) {
                                Ok(event) => {
                                    if !event.kline.is_closed {
                                        continue;
                                    }

                                    // P2 : rejeter les timestamps invalides au lieu de retourner l'epoch
                                    let open_time = match DateTime::from_timestamp_millis(
                                        event.kline.open_time,
                                    ) {
                                        Some(t) => t,
                                        None => {
                                            warn!(
                                                "open_time invalide ({}), bougie ignorée",
                                                event.kline.open_time
                                            );
                                            continue;
                                        }
                                    };
                                    let close_time = match DateTime::from_timestamp_millis(
                                        event.kline.close_time,
                                    ) {
                                        Some(t) => t,
                                        None => {
                                            warn!(
                                                "close_time invalide ({}), bougie ignorée",
                                                event.kline.close_time
                                            );
                                            continue;
                                        }
                                    };

                                    // P4 : rejeter les prix non parseable ou nuls/négatifs
                                    let open: f64 = match event.kline.open.parse() {
                                        Ok(v) if v > 0.0 => v,
                                        _ => {
                                            warn!(
                                                "Prix open invalide '{}', bougie ignorée",
                                                event.kline.open
                                            );
                                            continue;
                                        }
                                    };
                                    let high: f64 = match event.kline.high.parse() {
                                        Ok(v) if v > 0.0 => v,
                                        _ => {
                                            warn!(
                                                "Prix high invalide '{}', bougie ignorée",
                                                event.kline.high
                                            );
                                            continue;
                                        }
                                    };
                                    let low: f64 = match event.kline.low.parse() {
                                        Ok(v) if v > 0.0 => v,
                                        _ => {
                                            warn!(
                                                "Prix low invalide '{}', bougie ignorée",
                                                event.kline.low
                                            );
                                            continue;
                                        }
                                    };
                                    let close: f64 = match event.kline.close.parse() {
                                        Ok(v) if v > 0.0 => v,
                                        _ => {
                                            warn!(
                                                "Prix close invalide '{}', bougie ignorée",
                                                event.kline.close
                                            );
                                            continue;
                                        }
                                    };
                                    let volume: f64 =
                                        event.kline.volume.parse().unwrap_or(0.0);

                                    let candle = Candle {
                                        open_time,
                                        close_time,
                                        open,
                                        high,
                                        low,
                                        close,
                                        volume,
                                        is_closed: true,
                                    };

                                    // P5 : try_send évite de bloquer le WebSocket si le channel est plein
                                    match tx.try_send(candle) {
                                        Ok(_) => {}
                                        Err(mpsc::error::TrySendError::Full(_)) => {
                                            warn!("Channel saturé — bougie droppée (traitement trop lent)");
                                        }
                                        Err(mpsc::error::TrySendError::Closed(_)) => {
                                            return Ok(());
                                        }
                                    }
                                }
                                Err(e) => warn!("Impossible de parser le message kline: {}", e),
                            }
                        }
                        Ok(Message::Ping(_)) => {}
                        Ok(Message::Close(_)) => {
                            warn!("WebSocket closed, reconnecting...");
                            break;
                        }
                        Err(e) => {
                            error!("WebSocket error: {}", e);
                            break;
                        }
                        _ => {}
                    }
                }
            }
            Ok(Err(e)) => {
                error!("Échec connexion WebSocket Binance: {}", e);
            }
            Err(_) => {
                error!("Timeout connexion WebSocket Binance (15s)");
            }
        }

        tokio::time::sleep(Duration::from_secs(5)).await;
        info!("Reconnecting to Binance WebSocket...");
    }
}
