use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{interval as tick_interval, Duration};
use tracing::{info, warn};

use crate::logger::TradeLogger;
use crate::polymarket::PolymarketClient;
use crate::strategy::Prediction;

pub fn build_signal_key(strategy_name: &str, slug: &str, prediction: &Prediction) -> String {
    format!(
        "{}:{}:{}",
        strategy_name.trim().to_ascii_lowercase(),
        slug.trim().to_ascii_lowercase(),
        prediction.to_string().to_ascii_uppercase()
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PendingTrade {
    trade_id: String,
    order_id: String,
    signal_key: String,
}

/// Suit les ordres ouverts et met à jour leur `outcome` dans le CSV dès qu'ils
/// atteignent un état terminal (MATCHED / FILLED / CANCELLED / EXPIRED).
///
/// Les ordres dry-run (id préfixé par "dry-run-") sont ignorés silencieusement.
pub struct PositionTracker {
    pending: Mutex<Vec<PendingTrade>>,
    client: Arc<PolymarketClient>,
    logger: Arc<TradeLogger>,
    state_path: PathBuf,
}

impl PositionTracker {
    pub fn new(client: Arc<PolymarketClient>, logger: Arc<TradeLogger>, logs_dir: &str) -> Self {
        let state_path = PathBuf::from(logs_dir).join("pending_orders.json");
        let pending = Self::load_pending(&state_path);
        if !pending.is_empty() {
            info!(
                "[TRACKER] {} ordres rechargés depuis {}",
                pending.len(),
                state_path.display()
            );
        }
        Self {
            pending: Mutex::new(pending),
            client,
            logger,
            state_path,
        }
    }

    /// Enregistre un ordre pour suivi. Les ordres dry-run sont ignorés.
    pub async fn track(&self, trade_id: String, order_id: String, signal_key: String) {
        if order_id.starts_with("dry-run-") {
            return;
        }
        let mut pending = self.pending.lock().await;
        if pending.iter().any(|t| t.signal_key == signal_key || t.order_id == order_id) {
            warn!(
                "[TRACKER] Suivi déjà actif | trade_id={} order_id={} signal_key={}",
                trade_id, order_id, signal_key
            );
            return;
        }
        info!(
            "[TRACKER] Suivi activé | trade_id={} order_id={} signal_key={}",
            trade_id, order_id, signal_key
        );
        pending.push(PendingTrade {
            trade_id,
            order_id,
            signal_key,
        });
        if let Err(e) = self.save_pending(&pending) {
            warn!("[TRACKER] Sauvegarde état tracker échouée: {}", e);
        }
    }

    pub async fn is_signal_active(&self, signal_key: &str) -> bool {
        self.pending
            .lock()
            .await
            .iter()
            .any(|trade| trade.signal_key == signal_key)
    }

    pub async fn pending_count(&self) -> usize {
        self.pending.lock().await.len()
    }

    /// Boucle de polling en arrière-plan (toutes les 30 secondes).
    /// À lancer avec `tokio::spawn`.
    pub async fn run_poll_loop(self: Arc<Self>) {
        let mut ticker = tick_interval(Duration::from_secs(30));
        loop {
            ticker.tick().await;
            let pending_count = self.pending.lock().await.len();
            if pending_count == 0 {
                continue;
            }
            info!("[TRACKER] Polling {} ordres ouverts…", pending_count);
            if let Err(e) = self.poll_once().await {
                warn!("[TRACKER] Erreur de polling: {}", e);
            }
        }
    }

    async fn poll_once(&self) -> anyhow::Result<()> {
        let mut pending = self.pending.lock().await;
        let mut still_pending = Vec::new();

        for trade in pending.drain(..) {
            match self.client.get_order_status(&trade.order_id).await {
                Ok(status) => {
                    info!(
                        "[TRACKER] trade_id={} order_id={} status={}",
                        trade.trade_id, trade.order_id, status
                    );
                    let is_terminal = matches!(
                        status.as_str(),
                        "MATCHED" | "FILLED" | "CANCELLED" | "EXPIRED" | "UNMATCHED"
                    );
                    if is_terminal {
                        if let Err(e) = self.logger.update_outcome(&trade.trade_id, &status) {
                            warn!("[TRACKER] update_outcome failed: {}", e);
                        }
                    } else {
                        still_pending.push(trade);
                    }
                }
                Err(e) => {
                    warn!("[TRACKER] get_order_status({}) failed: {}", trade.order_id, e);
                    still_pending.push(trade);
                }
            }
        }

        *pending = still_pending;
        if let Err(e) = self.save_pending(&pending) {
            warn!("[TRACKER] Sauvegarde état tracker échouée: {}", e);
        }
        Ok(())
    }

    fn load_pending(state_path: &PathBuf) -> Vec<PendingTrade> {
        match fs::read_to_string(state_path) {
            Ok(content) => match serde_json::from_str::<Vec<PendingTrade>>(&content) {
                Ok(pending) => pending,
                Err(e) => {
                    warn!(
                        "[TRACKER] pending_orders.json invalide ({}): {}",
                        state_path.display(),
                        e
                    );
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        }
    }

    fn save_pending(&self, pending: &[PendingTrade]) -> Result<()> {
        let body = serde_json::to_string_pretty(pending)?;
        fs::write(&self.state_path, body)?;
        Ok(())
    }
}
