use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::config::{Config, ExecutionMode};
use crate::strategy::{Prediction, Signal};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketInfo {
    pub condition_id: String,
    pub up_token_id: String,
    pub down_token_id: String,
    pub slug: String,
}

#[derive(Debug, Clone)]
pub struct OrderResult {
    pub order_id: String,
    pub status: String,
    #[allow(dead_code)]
    pub submitted_at: DateTime<Utc>,
    pub ack_at: DateTime<Utc>,
}

pub struct PolymarketClient {
    config: Config,
    // Utilisé en V2 pour les appels HTTP réels
    #[allow(dead_code)]
    http: reqwest::Client,
}

impl PolymarketClient {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    /// Construit le slug Polymarket depuis le timestamp d'ouverture de la bougie cible.
    /// Format approximatif : btc-updown-5m-<YYYYMMDD>
    /// NOTE: le format exact doit être validé contre l'API live Polymarket Gamma.
    pub fn build_slug(open_time_ms: i64) -> String {
        let dt = DateTime::from_timestamp_millis(open_time_ms).unwrap_or_default();
        format!("btc-updown-5m-{}", dt.format("%Y%m%d"))
    }

    /// Résout slug -> condition_id + tokenIds UP/DOWN via Polymarket Gamma API.
    /// STUB V1 — à implémenter en V2 :
    ///   GET https://gamma-api.polymarket.com/markets?slug={slug}
    pub async fn resolve_market(&self, slug: &str) -> Result<MarketInfo> {
        warn!(
            "[STUB] resolve_market non implémenté (V2). Slug: {}. \
             Utilisez l'API Gamma pour résoudre les tokenIds UP/DOWN.",
            slug
        );
        Ok(MarketInfo {
            condition_id: "STUB_CONDITION_ID".to_string(),
            up_token_id: "STUB_UP_TOKEN_ID".to_string(),
            down_token_id: "STUB_DOWN_TOKEN_ID".to_string(),
            slug: slug.to_string(),
        })
    }

    /// Place un ordre sur Polymarket selon le signal reçu.
    /// En mode DryRun : simule sans appel réseau.
    /// En mode Market/Limit (V2) : nécessite signature EIP-712 avec clé privée EVM.
    pub async fn place_order(&self, signal: &Signal, market: &MarketInfo) -> Result<OrderResult> {
        let token_id = match &signal.prediction {
            Prediction::Up => &market.up_token_id,
            Prediction::Down => &market.down_token_id,
        };

        let order_type = match self.config.execution_mode {
            ExecutionMode::Market => "MARKET",
            ExecutionMode::Limit => "LIMIT",
            ExecutionMode::DryRun => "DRY_RUN",
        };

        let submitted_at = Utc::now();

        match self.config.execution_mode {
            ExecutionMode::DryRun => {
                info!(
                    "[DRY-RUN] Ordre simulé | type={} token={} side=BUY amount={} USDC",
                    order_type, token_id, self.config.trade_amount_usdc
                );
                Ok(OrderResult {
                    order_id: format!("dry-run-{}", uuid::Uuid::new_v4()),
                    status: "DRY_RUN".to_string(),
                    submitted_at,
                    ack_at: Utc::now(),
                })
            }
            _ => {
                // STUB V2 — implémentation réelle :
                // 1. GET /markets?slug=... pour récupérer condition_id + tokenIds
                // 2. Construire l'ordre signé EIP-712 avec la clé privée EVM
                // 3. POST /order avec le payload signé
                // Ref: https://docs.polymarket.com/#place-order
                warn!(
                    "[STUB] Ordre réel non implémenté (V2). \
                     Nécessite signature EIP-712 et clé privée EVM configurée."
                );
                Ok(OrderResult {
                    order_id: format!("stub-{}", uuid::Uuid::new_v4()),
                    status: "STUB".to_string(),
                    submitted_at,
                    ack_at: Utc::now(),
                })
            }
        }
    }
}
