use tracing::debug;

use crate::binance::Candle;
use crate::strategy::{Prediction, Signal, Strategy};

const RSI_PERIOD: usize = 7;
const STREAK: usize = 3;

/// Couleur stricte d'une bougie : NEUTRE si doji (close == open).
/// Identique à la fonction Python `candle_color`.
fn strict_color(c: &Candle) -> &'static str {
    if c.close > c.open {
        "VERTE"
    } else if c.close < c.open {
        "ROUGE"
    } else {
        "NEUTRE"
    }
}

fn rsi_from_avgs(avg_gain: f64, avg_loss: f64) -> f64 {
    if avg_loss == 0.0 {
        return 100.0;
    }
    let rs = avg_gain / avg_loss;
    100.0 - 100.0 / (1.0 + rs)
}

/// RSI de Wilder (lissé EMA) — identique au script Python de référence.
///
/// Phase seed  : les RSI_PERIOD premiers deltas → moyenne simple (SMA).
/// Phase live  : chaque delta suivant → lissage exponentiel de Wilder :
///   avg_gain = (avg_gain * (period-1) + gain) / period
pub struct ThreeCandleRsi7Reversal {
    /// Dernières STREAK bougies pour la détection de série.
    recent: Vec<Candle>,
    /// Dernier close vu (nécessaire pour calculer le delta).
    last_close: Option<f64>,
    /// Moyennes lissées Wilder (None avant la fin du seed).
    avg_gain: Option<f64>,
    avg_loss: Option<f64>,
    /// Accumulation des gains/pertes pendant la phase seed.
    seed_gains: Vec<f64>,
    seed_losses: Vec<f64>,
    /// RSI courant (None tant que le seed n'est pas terminé).
    rsi: Option<f64>,
}

impl ThreeCandleRsi7Reversal {
    pub fn new() -> Self {
        Self {
            recent: Vec::with_capacity(STREAK + 1),
            last_close: None,
            avg_gain: None,
            avg_loss: None,
            seed_gains: Vec::with_capacity(RSI_PERIOD),
            seed_losses: Vec::with_capacity(RSI_PERIOD),
            rsi: None,
        }
    }

    /// Alimente l'état interne (RSI + fenêtre de série) avec une nouvelle bougie.
    fn feed_candle(&mut self, candle: &Candle) {
        if let Some(last) = self.last_close {
            let change = candle.close - last;
            let gain = change.max(0.0);
            let loss = (-change).max(0.0);

            if self.avg_gain.is_none() {
                // Phase seed : on accumule jusqu'à RSI_PERIOD deltas
                self.seed_gains.push(gain);
                self.seed_losses.push(loss);
                if self.seed_gains.len() == RSI_PERIOD {
                    let ag = self.seed_gains.iter().sum::<f64>() / RSI_PERIOD as f64;
                    let al = self.seed_losses.iter().sum::<f64>() / RSI_PERIOD as f64;
                    self.avg_gain = Some(ag);
                    self.avg_loss = Some(al);
                    self.rsi = Some(rsi_from_avgs(ag, al));
                }
            } else {
                // Phase live : lissage exponentiel de Wilder
                let ag = (self.avg_gain.unwrap() * (RSI_PERIOD - 1) as f64 + gain)
                    / RSI_PERIOD as f64;
                let al = (self.avg_loss.unwrap() * (RSI_PERIOD - 1) as f64 + loss)
                    / RSI_PERIOD as f64;
                self.avg_gain = Some(ag);
                self.avg_loss = Some(al);
                self.rsi = Some(rsi_from_avgs(ag, al));
            }
        }
        self.last_close = Some(candle.close);

        self.recent.push(candle.clone());
        if self.recent.len() > STREAK {
            self.recent.remove(0);
        }
    }

    /// RSI courant (None tant que RSI_PERIOD deltas n'ont pas été vus).
    pub fn compute_rsi(&self) -> Option<f64> {
        self.rsi
    }

    /// Some(true)  = 3 bougies VERTE consécutives (close > open)
    /// Some(false) = 3 bougies ROUGE consécutives (close < open)
    /// None        = série mixte ou doji présent
    pub fn last_three_same_color(&self) -> Option<bool> {
        if self.recent.len() < STREAK {
            return None;
        }
        let colors: Vec<&str> = self.recent.iter().map(strict_color).collect();
        if colors.iter().all(|&c| c == "VERTE") {
            Some(true)
        } else if colors.iter().all(|&c| c == "ROUGE") {
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

    fn warmup(&mut self, candle: &Candle) {
        self.feed_candle(candle);
    }

    fn on_closed_candle(&mut self, candle: &Candle) -> Option<Signal> {
        self.feed_candle(candle);
        debug!(
            "[STRATEGY] rsi={:?} série={:?}",
            self.rsi,
            self.last_three_same_color()
        );

        let rsi = self.rsi?;
        let is_green_series = self.last_three_same_color()?;
        let last = self.recent.last()?;

        let prediction = if is_green_series {
            // 3 VERTE + RSI suracheté => reversal DOWN
            if rsi >= 65.0 {
                Some(Prediction::Down)
            } else {
                None
            }
        } else {
            // 3 ROUGE + RSI survendu => reversal UP
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

    fn current_rsi(&self) -> Option<f64> {
        self.rsi
    }

    fn current_series(&self) -> Option<bool> {
        self.last_three_same_color()
    }
}
