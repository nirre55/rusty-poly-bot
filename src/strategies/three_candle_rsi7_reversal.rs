use tracing::debug;

use crate::binance::Candle;
use crate::strategy::{Prediction, Signal, Strategy};

const RSI_PERIOD: usize = 7;
/// Historique suffisant pour RSI7 (8 deltas) + 3 bougies de detection
const CANDLE_HISTORY: usize = RSI_PERIOD + 1 + 3;

pub struct ThreeCandleRsi7Reversal {
    candles: Vec<Candle>,
}

impl ThreeCandleRsi7Reversal {
    pub fn new() -> Self {
        Self {
            candles: Vec::with_capacity(CANDLE_HISTORY + 1),
        }
    }

    /// RSI simple (non lissé) sur les RSI_PERIOD derniers deltas.
    pub fn compute_rsi(&self) -> Option<f64> {
        if self.candles.len() < RSI_PERIOD + 1 {
            return None;
        }
        let recent = &self.candles[self.candles.len() - RSI_PERIOD - 1..];
        let mut gains = 0.0f64;
        let mut losses = 0.0f64;

        for i in 1..recent.len() {
            let diff = recent[i].close - recent[i - 1].close;
            if diff > 0.0 {
                gains += diff;
            } else {
                losses += diff.abs();
            }
        }

        let avg_gain = gains / RSI_PERIOD as f64;
        let avg_loss = losses / RSI_PERIOD as f64;

        // P1 : marché complètement plat (avg_gain=0 ET avg_loss=0) → RSI neutre à 50
        // (et non 100 qui signifierait une tendance haussière totale)
        if avg_loss == 0.0 {
            return if avg_gain == 0.0 {
                Some(50.0)
            } else {
                Some(100.0)
            };
        }
        let rs = avg_gain / avg_loss;
        Some(100.0 - 100.0 / (1.0 + rs))
    }

    /// Retourne Some(true) si les 3 dernières bougies sont toutes vertes,
    /// Some(false) si toutes rouges, None sinon.
    pub fn last_three_same_color(&self) -> Option<bool> {
        if self.candles.len() < 3 {
            return None;
        }
        let len = self.candles.len();
        let c1 = &self.candles[len - 3];
        let c2 = &self.candles[len - 2];
        let c3 = &self.candles[len - 1];

        if c1.is_green() && c2.is_green() && c3.is_green() {
            Some(true)
        } else if c1.is_red() && c2.is_red() && c3.is_red() {
            Some(false)
        } else {
            None
        }
    }
}

impl Strategy for ThreeCandleRsi7Reversal {
    fn name(&self) -> &str {
        "three_candle_rsi7_reversal"
    }

    fn on_closed_candle(&mut self, candle: &Candle) -> Option<Signal> {
        self.candles.push(candle.clone());
        if self.candles.len() > CANDLE_HISTORY {
            self.candles.remove(0);
        }

        let rsi = self.compute_rsi();
        let is_green_series = self.last_three_same_color();

        debug!(
            "[STRATEGY] candles={} RSI={} | série={}",
            self.candles.len(),
            rsi.map(|r| format!("{:.2}", r)).unwrap_or("N/A".to_string()),
            match is_green_series {
                Some(true) => "3xVERT",
                Some(false) => "3xROUGE",
                None => "mixte",
            }
        );

        let rsi = rsi?;
        let is_green_series = is_green_series?;
        let last = self.candles.last()?;

        let prediction = if is_green_series {
            // 3 bougies vertes + RSI suracheté => reversal DOWN
            if rsi >= 65.0 {
                Some(Prediction::Down)
            } else {
                None
            }
        } else {
            // 3 bougies rouges + RSI survendu => reversal UP
            if rsi <= 35.0 {
                Some(Prediction::Up)
            } else {
                None
            }
        }?;

        Some(Signal {
            prediction,
            signal_candle_close_time: last.close_time,
            rsi,
            strategy_name: self.name().to_string(),
        })
    }
}
