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

    /// RSI simple (non lisse) sur les RSI_PERIOD derniers deltas.
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

        if avg_loss == 0.0 {
            return Some(100.0);
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

        let rsi = self.compute_rsi()?;
        let is_green_series = self.last_three_same_color()?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_candle(open: f64, close: f64) -> Candle {
        Candle {
            open_time: Utc::now(),
            close_time: Utc::now(),
            open,
            high: open.max(close) + 1.0,
            low: open.min(close) - 1.0,
            close,
            volume: 1.0,
            is_closed: true,
        }
    }

    #[test]
    fn test_candle_color() {
        let green = make_candle(100.0, 110.0);
        let red = make_candle(110.0, 100.0);
        let doji = make_candle(100.0, 100.0);

        assert!(green.is_green());
        assert!(!green.is_red());
        assert!(red.is_red());
        assert!(!red.is_green());
        assert!(doji.is_green()); // doji compté comme vert
        assert!(!doji.is_red());
    }

    #[test]
    fn test_three_consecutive_same_color_detection() {
        let mut strategy = ThreeCandleRsi7Reversal::new();

        // Moins de 3 bougies => None
        strategy.candles.push(make_candle(100.0, 101.0));
        assert!(strategy.last_three_same_color().is_none());
        strategy.candles.push(make_candle(101.0, 102.0));
        assert!(strategy.last_three_same_color().is_none());

        // 3 vertes
        strategy.candles.push(make_candle(102.0, 103.0));
        assert_eq!(strategy.last_three_same_color(), Some(true));

        // 3 rouges
        strategy.candles.clear();
        strategy.candles.push(make_candle(103.0, 100.0));
        strategy.candles.push(make_candle(100.0, 98.0));
        strategy.candles.push(make_candle(98.0, 95.0));
        assert_eq!(strategy.last_three_same_color(), Some(false));

        // Mixte => None
        strategy.candles.clear();
        strategy.candles.push(make_candle(100.0, 101.0));
        strategy.candles.push(make_candle(101.0, 99.0));
        strategy.candles.push(make_candle(99.0, 100.0));
        assert!(strategy.last_three_same_color().is_none());
    }

    #[test]
    fn test_rsi_not_enough_candles() {
        let mut strategy = ThreeCandleRsi7Reversal::new();
        for i in 0..RSI_PERIOD {
            strategy.candles.push(make_candle(100.0 + i as f64, 101.0 + i as f64));
        }
        // 7 bougies => 6 deltas, pas assez pour RSI7 (besoin de 7 deltas)
        assert!(strategy.compute_rsi().is_none());

        strategy.candles.push(make_candle(107.0, 108.0));
        // 8 bougies => 7 deltas => RSI calculable
        assert!(strategy.compute_rsi().is_some());
    }

    #[test]
    fn test_rsi_only_gains_gives_100() {
        let mut strategy = ThreeCandleRsi7Reversal::new();
        // Toujours en hausse => RSI = 100
        for i in 0..=RSI_PERIOD {
            strategy.candles.push(make_candle(100.0 + i as f64, 101.0 + i as f64));
        }
        let rsi = strategy.compute_rsi().unwrap();
        assert_eq!(rsi, 100.0);
    }

    #[test]
    fn test_prediction_mapping() {
        assert_eq!(format!("{}", Prediction::Up), "UP");
        assert_eq!(format!("{}", Prediction::Down), "DOWN");
    }

    #[test]
    fn test_slug_construction() {
        // Le slug est construit depuis le timestamp d'ouverture de la bougie cible
        let ts_ms = 1710000000000i64;
        let dt = chrono::DateTime::from_timestamp_millis(ts_ms).unwrap();
        let slug = format!("btc-updown-5m-{}", dt.format("%Y%m%d"));
        assert!(slug.starts_with("btc-updown-5m-"));
        assert_eq!(slug, "btc-updown-5m-20240309");
    }

    #[test]
    fn test_no_signal_without_rsi_condition() {
        let mut strategy = ThreeCandleRsi7Reversal::new();
        // Seed 5 bougies rouges avec closes décroissants : 110, 108, 106, 104, 102
        // Cela crée de fortes pertes dans la fenêtre RSI.
        for &close in &[110.0f64, 108.0, 106.0, 104.0, 102.0] {
            strategy.candles.push(make_candle(close + 3.0, close));
        }
        // 3 bougies vertes : closes 104, 106, 108
        // Fenêtre RSI finale (8 candles) : closes [110,108,106,104,102,104,106,108]
        // Deltas : -2,-2,-2,-2,+2,+2,+2 => RSI ≈ 42.86 (entre 35 et 65 => pas de signal)
        let mut result = None;
        for &close in &[104.0f64, 106.0, 108.0] {
            result = strategy.on_closed_candle(&make_candle(close - 1.0, close));
        }
        assert!(result.is_none(), "RSI ≈ 42, pas de signal attendu malgré 3 bougies vertes");
    }
}
