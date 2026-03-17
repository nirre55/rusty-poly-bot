use crate::binance::Candle;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq)]
pub enum Prediction {
    Up,
    Down,
}

impl std::fmt::Display for Prediction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Prediction::Up => write!(f, "UP"),
            Prediction::Down => write!(f, "DOWN"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Signal {
    pub prediction: Prediction,
    pub signal_candle_close_time: DateTime<Utc>,
    pub rsi: f64,
    pub strategy_name: String,
}

/// Abstraction permettant de brancher plusieurs strategies.
/// Chaque strategie recoit les bougies fermees une par une
/// et retourne un signal optionnel.
pub trait Strategy: Send + Sync {
    fn name(&self) -> &str;
    fn on_closed_candle(&mut self, candle: &Candle) -> Option<Signal>;
    /// Alimente l'historique sans logger ni retourner de signal (préchargement).
    fn warmup(&mut self, candle: &Candle);
    /// RSI courant (None si pas assez de bougies).
    fn current_rsi(&self) -> Option<f64>;
    /// Série des 3 dernières bougies : Some(true)=3xVERT, Some(false)=3xROUGE, None=mixte.
    fn current_series(&self) -> Option<bool>;
}
