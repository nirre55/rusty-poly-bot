use std::collections::VecDeque;
use tracing::debug;

use crate::binance::Candle;
use crate::strategy::{Prediction, Signal, Strategy};

const RSI_PERIOD: usize = 7;
const STREAK: usize = 3;
const ATR_PERIOD: usize = 14;
const ATR_MULTIPLIER: f64 = 1.0;
const BODY_RATIO_MIN: f64 = 0.60;

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
    recent: VecDeque<Candle>,
    /// Dernier close vu (nécessaire pour calculer le delta RSI et le True Range).
    last_close: Option<f64>,
    /// Moyennes lissées Wilder (None avant la fin du seed).
    avg_gain: Option<f64>,
    avg_loss: Option<f64>,
    /// Accumulation des gains/pertes pendant la phase seed RSI.
    seed_gains: Vec<f64>,
    seed_losses: Vec<f64>,
    /// RSI courant (None tant que RSI_PERIOD deltas n'ont pas été vus).
    rsi: Option<f64>,
    /// Fenêtre glissante des ATR_PERIOD derniers True Ranges.
    true_ranges: VecDeque<f64>,
    /// ATR14 courant (None tant que ATR_PERIOD True Ranges n'ont pas été accumulés).
    atr: Option<f64>,
    /// Seuil RSI suracheté (signal DOWN si RSI >= ce seuil).
    rsi_overbought: f64,
    /// Seuil RSI survendu (signal UP si RSI <= ce seuil).
    rsi_oversold: f64,
}

impl ThreeCandleRsi7Reversal {
    pub fn new(rsi_overbought: f64, rsi_oversold: f64) -> Self {
        Self {
            recent: VecDeque::with_capacity(STREAK + 1),
            last_close: None,
            avg_gain: None,
            avg_loss: None,
            seed_gains: Vec::with_capacity(RSI_PERIOD),
            seed_losses: Vec::with_capacity(RSI_PERIOD),
            rsi: None,
            true_ranges: VecDeque::with_capacity(ATR_PERIOD + 1),
            atr: None,
            rsi_overbought,
            rsi_oversold,
        }
    }

    /// Alimente l'état interne (RSI + ATR + fenêtre de série) avec une nouvelle bougie.
    fn feed_candle(&mut self, candle: &Candle) {
        if let Some(last) = self.last_close {
            // --- RSI ---
            let change = candle.close - last;
            let gain = change.max(0.0);
            let loss = (-change).max(0.0);

            if self.avg_gain.is_none() {
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
                let ag = (self.avg_gain.unwrap() * (RSI_PERIOD - 1) as f64 + gain)
                    / RSI_PERIOD as f64;
                let al = (self.avg_loss.unwrap() * (RSI_PERIOD - 1) as f64 + loss)
                    / RSI_PERIOD as f64;
                self.avg_gain = Some(ag);
                self.avg_loss = Some(al);
                self.rsi = Some(rsi_from_avgs(ag, al));
            }

            // --- ATR14 (True Range = max des 3 mesures standard) ---
            let tr = (candle.high - candle.low)
                .max((candle.high - last).abs())
                .max((candle.low - last).abs());
            if self.true_ranges.len() == ATR_PERIOD {
                self.true_ranges.pop_front();
            }
            self.true_ranges.push_back(tr);
            if self.true_ranges.len() == ATR_PERIOD {
                self.atr = Some(self.true_ranges.iter().sum::<f64>() / ATR_PERIOD as f64);
            }
        }
        self.last_close = Some(candle.close);

        self.recent.push_back(candle.clone());
        if self.recent.len() > STREAK {
            self.recent.pop_front();
        }
    }

    /// RSI courant (None tant que RSI_PERIOD deltas n'ont pas été vus).
    pub fn compute_rsi(&self) -> Option<f64> {
        self.rsi
    }

    /// ATR14 courant (None tant que ATR_PERIOD bougies n'ont pas été vues).
    pub fn compute_atr(&self) -> Option<f64> {
        self.atr
    }

    /// Vérifie que le range de la bougie (high-low) est ≥ ATR_MULTIPLIER × ATR14.
    /// Retourne None si l'ATR n'est pas encore disponible.
    fn range_ok(&self, candle: &Candle) -> Option<bool> {
        let atr = self.atr?;
        // Tolérance relative 1e-9 pour absorber les erreurs d'arrondi IEEE-754
        // (ATR = moyenne de TR → légèrement différent du range direct à haute valeur)
        Some((candle.high - candle.low) >= ATR_MULTIPLIER * atr - atr * 1e-9)
    }

    /// Vérifie que le body ratio de la bougie est ≥ BODY_RATIO_MIN.
    /// body_ratio = |close - open| / (high - low)
    /// Retourne None si le range est nul (bougie plate).
    fn body_ratio_ok(candle: &Candle) -> Option<bool> {
        let range = candle.high - candle.low;
        if range == 0.0 {
            return None;
        }
        Some((candle.close - candle.open).abs() / range >= BODY_RATIO_MIN)
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
            "[STRATEGY] rsi={:?} atr={:?} série={:?}",
            self.rsi,
            self.atr,
            self.last_three_same_color()
        );

        let rsi = self.rsi?;
        let is_green_series = self.last_three_same_color()?;
        let last = self.recent.back()?;

        // Filtres Range et Body Ratio (silencieux si ATR pas encore prêt)
        if !self.range_ok(last).unwrap_or(false) {
            return None;
        }
        if !Self::body_ratio_ok(last).unwrap_or(false) {
            return None;
        }

        let prediction = if is_green_series {
            // 3 VERTE + RSI suracheté => reversal DOWN
            if rsi >= self.rsi_overbought {
                Some(Prediction::Down)
            } else {
                None
            }
        } else {
            // 3 ROUGE + RSI survendu => reversal UP
            if rsi <= self.rsi_oversold {
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

    fn current_atr(&self) -> Option<f64> {
        self.atr
    }
}
