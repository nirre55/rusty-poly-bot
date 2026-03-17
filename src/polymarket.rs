use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;
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
    // P6 : client HTTP avec timeout configuré, prêt pour la V2
    #[allow(dead_code)]
    http: reqwest::Client,
}

impl PolymarketClient {
    pub fn new(config: Config) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { config, http }
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
                    "[DRY-RUN] Ordre simulé | type={} token={} amount={} USDC",
                    order_type, token_id, self.config.trade_amount_usdc
                );
                Ok(OrderResult {
                    order_id: format!("dry-run-{}", uuid::Uuid::new_v4()),
                    status: "DRY_RUN".to_string(),
                    submitted_at,
                    ack_at: Utc::now(),
                })
            }
            // P3 : retourner Err au lieu de Ok(STUB) pour éviter un faux positif d'ordre placé
            _ => Err(anyhow!(
                "Mode {:?} non implémenté (V2). \
                 Nécessite signature EIP-712 et clé privée EVM configurée. \
                 Ref: https://docs.polymarket.com/#place-order",
                self.config.execution_mode
            )),
        }
    }
}
