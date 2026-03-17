use anyhow::Result;
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use serde::Deserialize;
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

pub async fn stream_candles(
    url: &str,
    symbol: &str,
    interval: &str,
    tx: mpsc::Sender<Candle>,
) -> Result<()> {
    let ws_url = format!("{}/{symbol}@kline_{interval}", url);
    info!("Connecting to Binance WebSocket: {}", ws_url);

    loop {
        match connect_async(&ws_url).await {
            Ok((ws_stream, _)) => {
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
                                    let candle = Candle {
                                        open_time: DateTime::from_timestamp_millis(
                                            event.kline.open_time,
                                        )
                                        .unwrap_or_default(),
                                        close_time: DateTime::from_timestamp_millis(
                                            event.kline.close_time,
                                        )
                                        .unwrap_or_default(),
                                        open: event.kline.open.parse().unwrap_or(0.0),
                                        high: event.kline.high.parse().unwrap_or(0.0),
                                        low: event.kline.low.parse().unwrap_or(0.0),
                                        close: event.kline.close.parse().unwrap_or(0.0),
                                        volume: event.kline.volume.parse().unwrap_or(0.0),
                                        is_closed: true,
                                    };
                                    if tx.send(candle).await.is_err() {
                                        return Ok(());
                                    }
                                }
                                Err(e) => warn!("Failed to parse kline event: {}", e),
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
            Err(e) => {
                error!("Failed to connect to Binance WebSocket: {}", e);
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        info!("Reconnecting to Binance WebSocket...");
    }
}
