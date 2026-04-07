use alloy::primitives::{keccak256, Address, B256, U256};
use alloy::signers::{local::PrivateKeySigner, Signer};
use anyhow::{anyhow, Result};
use backoff::{future::retry, ExponentialBackoffBuilder};
use base64::{engine::general_purpose::URL_SAFE, Engine};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use polymarket_client_sdk::auth::state::Authenticated;
use polymarket_client_sdk::auth::Normal;
use polymarket_client_sdk::clob::{Client as SdkClobClient, Config as SdkConfig};
use polymarket_client_sdk::clob::types::{
    Amount,
    OrderType as SdkOrderType,
    Side as SdkSide,
    SignatureType as SdkSignatureType,
};
use polymarket_client_sdk::types::Decimal;
use polymarket_client_sdk::POLYGON;
use serde::{Deserialize, Serialize};
use serde_json;
use sha2::Sha256;
use std::{str::FromStr, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::config::{Config, ExecutionMode};
use crate::strategy::{Prediction, Signal};

// ── Constantes ───────────────────────────────────────────────────────────────

const GAMMA_API_BASE: &str = "https://gamma-api.polymarket.com";
const CLOB_API_BASE: &str = "https://clob.polymarket.com";
const CTF_EXCHANGE_ADDR: &str = "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E";
const POLYGON_CHAIN_ID: u64 = 137;
const CLOB_AUTH_MSG: &str = "This message attests that I control the given wallet";
const FOK_RETRY_DELAYS_SECS: [u64; 3] = [3, 7, 10];

// ── Types publics (API inchangée) ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketInfo {
    pub condition_id: String,
    pub up_token_id: String,
    pub down_token_id: String,
    pub slug: String,
    /// Taille minimale d'un ordre en shares (ex: 5.0 = 5 shares minimum)
    pub order_min_size: f64,
}

#[derive(Debug, Clone)]
pub struct OrderResult {
    pub order_id: String,
    pub status: String,
    #[allow(dead_code)]
    pub submitted_at: DateTime<Utc>,
    pub ack_at: DateTime<Utc>,
}

// ── Types internes ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct GammaMarket {
    #[serde(alias = "conditionId")]
    condition_id: String,
    /// JSON-encodé : "[\"Up\", \"Down\"]"
    outcomes: String,
    /// JSON-encodé : "[\"<token_id_up>\", \"<token_id_down>\"]"
    #[serde(alias = "clobTokenIds")]
    clob_token_ids: String,
    /// Taille minimale d'un ordre en shares
    #[serde(alias = "orderMinSize", default = "default_order_min_size")]
    order_min_size: f64,
}

fn default_order_min_size() -> f64 {
    5.0
}

#[derive(Debug, Clone)]
struct ApiCreds {
    api_key: String,
    secret: String,
    passphrase: String,
    address: String,
}

#[derive(Deserialize)]
struct ApiKeyResponse {
    #[serde(rename = "apiKey")]
    api_key: String,
    secret: String,
    passphrase: String,
}

#[allow(dead_code)]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OrderPayload {
    salt: String,
    maker: String,
    signer: String,
    taker: String,
    token_id: String,
    maker_amount: String,
    taker_amount: String,
    expiration: String,
    nonce: String,
    fee_rate_bps: String,
    side: String,
    signature_type: u8,   // entier, pas string (Polymarket Python client: signatureType = 0)
    signature: String,
}

#[allow(dead_code)]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PostOrderBody {
    order: OrderPayload,
    owner: String,
    order_type: String,
    post_only: bool,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct PostOrderResponse {
    #[serde(default)]
    success: bool,
    #[serde(rename = "orderID", default)]
    order_id: String,
    #[serde(default)]
    status: String,
    /// Polymarket renvoie "errorMsg" sur les succès partiels et "error" sur les rejets
    #[serde(rename = "errorMsg", alias = "error", default)]
    error_msg: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct TickSizeResponse {
    minimum_tick_size: f64,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct FeeRateResponse {
    #[serde(default)]
    base_fee: u64,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct OrderBookLevel {
    price: String,
    size: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct OrderBookResponse {
    #[serde(default)]
    asks: Vec<OrderBookLevel>,
    #[serde(default)]
    bids: Vec<OrderBookLevel>,
}

// ── Client ────────────────────────────────────────────────────────────────────

pub struct PolymarketClient {
    config: Config,
    http: reqwest::Client,
    signer: Option<Arc<PrivateKeySigner>>,
    api_creds: Mutex<Option<ApiCreds>>,
    /// Cache (slug → MarketInfo) : un marché actif à la fois, renouvelé si le slug change.
    market_cache: Mutex<Option<(String, MarketInfo)>>,
    /// Client SDK authentifié, créé une seule fois et réutilisé pour tous les ordres.
    /// Conserve les caches internes (tick_size, fee_rate_bps) entre les appels.
    sdk_client: Mutex<Option<SdkClobClient<Authenticated<Normal>>>>,
    /// Signer SDK pré-construit avec chain_id, réutilisé pour signer les ordres.
    sdk_signer: Option<PrivateKeySigner>,
}

impl PolymarketClient {
    pub fn new(config: Config) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .tcp_keepalive(Some(Duration::from_secs(30)))
            .pool_max_idle_per_host(4)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let signer = config.evm_private_key.as_deref().and_then(|pk| {
            let pk = pk.trim_start_matches("0x");
            match PrivateKeySigner::from_str(pk) {
                Ok(s) => Some(Arc::new(s)),
                Err(e) => {
                    warn!("POLYMARKET_PRIVATE_KEY invalide — mode réel désactivé: {}", e);
                    None
                }
            }
        });

        // Pré-construire le signer SDK avec chain_id pour réutilisation
        let sdk_signer = config.evm_private_key.as_deref().and_then(|pk| {
            let pk = pk.trim_start_matches("0x");
            PrivateKeySigner::from_str(pk)
                .ok()
                .map(|s| s.with_chain_id(Some(POLYGON)))
        });

        Self {
            config,
            http,
            signer,
            api_creds: Mutex::new(None),
            market_cache: Mutex::new(None),
            sdk_client: Mutex::new(None),
            sdk_signer,
        }
    }

    /// Pré-chauffe la connexion TCP/TLS vers le CLOB (payer le handshake une seule fois).
    /// À appeler dans `main()` avant la boucle de trading.
    pub async fn warm_up(&self) {
        match self.http.get(format!("{}/ok", CLOB_API_BASE)).send().await {
            Ok(_) => info!("Connexion CLOB Polymarket pré-chauffée"),
            Err(e) => warn!("warm_up CLOB échoué (non bloquant): {}", e),
        }
        // Pré-créer le client SDK authentifié pour que le premier ordre soit aussi rapide que les suivants.
        match self.get_or_create_sdk_client().await {
            Ok(_) => info!("Client SDK Polymarket pré-authentifié"),
            Err(e) => warn!("warm_up SDK échoué (non bloquant): {}", e),
        }
    }

    /// Construit le slug Polymarket depuis le timestamp d'ouverture de la bougie cible.
    /// Format : `{prefix}-<UNIX_TIMESTAMP_SECONDES>`
    /// Exemple : "btc-updown-5m-1742256301"
    pub fn build_slug(prefix: &str, open_time_ms: i64) -> String {
        let unix_secs = open_time_ms / 1000;
        format!("{}-{}", prefix, unix_secs)
    }

    /// Résout slug → condition_id + tokenIds UP/DOWN via l'API Gamma Polymarket.
    /// Résultat mis en cache : un seul appel réseau par slug distinct.
    pub async fn resolve_market(&self, slug: &str) -> Result<MarketInfo> {
        use std::time::Instant;

        {
            let cache = self.market_cache.lock().await;
            if let Some((cached_slug, info)) = cache.as_ref() {
                if cached_slug == slug {
                    return Ok(info.clone());
                }
            }
        }

        let t_resolve = Instant::now();
        let url = format!("{}/markets?slug={}", GAMMA_API_BASE, slug);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| anyhow!("Gamma API GET échoué: {}", e))?;
        let gamma_http_ms = t_resolve.elapsed().as_millis();

        if !resp.status().is_success() {
            return Err(anyhow!(
                "Gamma API {} → HTTP {}",
                url,
                resp.status()
            ));
        }

        let body = resp
            .text()
            .await
            .map_err(|e| anyhow!("Gamma API lecture body: {}", e))?;

        let markets: Vec<GammaMarket> = serde_json::from_str(&body)
            .map_err(|e| anyhow!("Gamma API parse JSON: {} | body={}", e, &body[..body.len().min(300)]))?;

        let market = markets
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("Aucun marché trouvé pour le slug '{}'", slug))?;

        // outcomes et clobTokenIds sont des strings JSON encodées (ex: "[\"Up\",\"Down\"]")
        let outcomes: Vec<String> = serde_json::from_str(&market.outcomes)
            .map_err(|e| anyhow!("Impossible de parser outcomes: {}", e))?;
        let token_ids: Vec<String> = serde_json::from_str(&market.clob_token_ids)
            .map_err(|e| anyhow!("Impossible de parser clobTokenIds: {}", e))?;

        let up_idx = outcomes
            .iter()
            .position(|o| o.eq_ignore_ascii_case("up"))
            .ok_or_else(|| anyhow!("Outcome 'Up' introuvable pour le slug '{}'", slug))?;
        let down_idx = outcomes
            .iter()
            .position(|o| o.eq_ignore_ascii_case("down"))
            .ok_or_else(|| anyhow!("Outcome 'Down' introuvable pour le slug '{}'", slug))?;

        let up_token_id = token_ids
            .get(up_idx)
            .ok_or_else(|| anyhow!("Token UP manquant dans clobTokenIds pour '{}'", slug))?
            .clone();
        let down_token_id = token_ids
            .get(down_idx)
            .ok_or_else(|| anyhow!("Token DOWN manquant dans clobTokenIds pour '{}'", slug))?
            .clone();

        let info = MarketInfo {
            condition_id: market.condition_id,
            up_token_id,
            down_token_id,
            slug: slug.to_string(),
            order_min_size: market.order_min_size,
        };

        debug!(
            "Marché résolu: slug={} condition_id={} UP={} DOWN={} | timing: gamma_http={}ms total={}ms",
            slug, info.condition_id, info.up_token_id, info.down_token_id,
            gamma_http_ms, t_resolve.elapsed().as_millis()
        );
        *self.market_cache.lock().await = Some((slug.to_string(), info.clone()));
        Ok(info)
    }

    /// Pré-chauffe les caches SDK (tick_size, fee_rate_bps, neg_risk) pour les tokens
    /// d'un marché résolu. À appeler après resolve_market pour que build() soit instantané.
    pub async fn warm_sdk_caches(&self, market: &MarketInfo) {
        let client = match self.get_or_create_sdk_client().await {
            Ok(c) => c,
            Err(_) => return,
        };
        // Pré-fetch tick_size + fee_rate_bps + neg_risk pour les deux tokens (UP et DOWN).
        // Les erreurs sont ignorées — ce n'est qu'un warm-up.
        for token_id in [&market.up_token_id, &market.down_token_id] {
            let _ = client.tick_size(token_id).await;
            let _ = client.fee_rate_bps(token_id).await;
        }
    }

    /// Place un ordre sur Polymarket selon le signal reçu.
    ///
    /// - `DryRun` : simule sans appel réseau (aucune clé requise).
    /// - `Market` : ordre FAK signé EIP-712 + headers HMAC-SHA256 L2.
    /// - `Limit`  : non implémenté.
    pub async fn place_order(&self, signal: &Signal, market: &MarketInfo) -> Result<OrderResult> {
        let token_id_str = match &signal.prediction {
            Prediction::Up => &market.up_token_id,
            Prediction::Down => &market.down_token_id,
        };

        let submitted_at = Utc::now();

        match self.config.execution_mode {
            ExecutionMode::DryRun => {
                info!(
                    "[DRY-RUN] Ordre simulé | type=FAK token={} amount={} USDC",
                    token_id_str, self.config.trade_amount_usdc
                );
                Ok(OrderResult {
                    order_id: format!("dry-run-{}", Uuid::new_v4()),
                    status: "DRY_RUN".to_string(),
                    submitted_at,
                    ack_at: Utc::now(),
                })
            }

            ExecutionMode::Market => {
                self.submit_market_order_with_retry(token_id_str, submitted_at)
                    .await
            }

            ExecutionMode::Limit => Err(anyhow!(
                "Mode Limit non implémenté — devra imposer un minimum de 5 shares"
            )),
        }
    }

    /// Récupère le statut courant d'un ordre via `GET /orders/{order_id}`.
    /// Requiert le signer (mode Market uniquement — les ordres dry-run ne sont pas tracés).
    pub async fn get_order_status(&self, order_id: &str) -> Result<String> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| anyhow!("get_order_status requiert POLYMARKET_PRIVATE_KEY"))?
            .clone();

        let creds = self.get_or_derive_creds(&signer).await?;
        let timestamp = Utc::now().timestamp().to_string();
        let path = format!("/data/order/{}", order_id);
        let sig = Self::compute_hmac_sig(&creds.secret, &timestamp, "GET", &path, "")?;

        #[derive(Deserialize)]
        struct OrderStatusResp {
            #[serde(default)]
            status: String,
        }

        let resp = self
            .http
            .get(format!("{}{}", CLOB_API_BASE, path))
            .header("POLY_ADDRESS", &creds.address)
            .header("POLY_API_KEY", &creds.api_key)
            .header("POLY_PASSPHRASE", &creds.passphrase)
            .header("POLY_SIGNATURE", &sig)
            .header("POLY_TIMESTAMP", &timestamp)
            .send()
            .await
            .map_err(|e| anyhow!("GET /data/order/{}: {}", order_id, e))?;

        if !resp.status().is_success() {
            return Err(anyhow!("GET /data/order/{} → HTTP {}", order_id, resp.status()));
        }

        let body: OrderStatusResp = resp
            .json()
            .await
            .map_err(|e| anyhow!("parse order status: {}", e))?;

        Ok(body.status)
    }

    // ── Helpers privés ────────────────────────────────────────────────────────

    /// Retourne les credentials API, les dérivant via L1 si pas encore en cache.
    async fn get_or_derive_creds(&self, signer: &PrivateKeySigner) -> Result<ApiCreds> {
        let mut guard = self.api_creds.lock().await;
        if let Some(creds) = guard.as_ref() {
            return Ok(creds.clone());
        }
        let creds = Self::derive_api_creds(&self.http, signer).await?;
        *guard = Some(creds.clone());
        Ok(creds)
    }

    /// Auth L1 : signe le message ClobAuth (EIP-712) et appelle POST /auth/api-key.
    async fn derive_api_creds(
        http: &reqwest::Client,
        signer: &PrivateKeySigner,
    ) -> Result<ApiCreds> {
        let timestamp = Utc::now().timestamp().to_string();
        let address = signer.address();
        let address_str = format!("{}", address);

        let signing_hash = Self::clob_auth_signing_hash(address, &timestamp, 0)?;
        let sig = signer
            .sign_hash(&signing_hash)
            .await
            .map_err(|e| anyhow!("ClobAuth signing: {:?}", e))?;
        let sig_hex = Self::sig_to_hex(&sig);

        // Essaye POST (créer), si 4xx essaye GET (récupérer une clé existante)
        let resp = http
            .post(format!("{}/auth/api-key", CLOB_API_BASE))
            .header("POLY_ADDRESS", &address_str)
            .header("POLY_SIGNATURE", &sig_hex)
            .header("POLY_TIMESTAMP", &timestamp)
            .header("POLY_NONCE", "0")
            .send()
            .await
            .map_err(|e| anyhow!("POST /auth/api-key: {}", e))?;

        let key_resp: ApiKeyResponse = if resp.status().is_success() {
            resp.json().await.map_err(|e| anyhow!("parse api-key response: {}", e))?
        } else {
            // Clé déjà existante — la récupérer avec GET
            let resp2 = http
                .get(format!("{}/auth/derive-api-key", CLOB_API_BASE))
                .header("POLY_ADDRESS", &address_str)
                .header("POLY_SIGNATURE", &sig_hex)
                .header("POLY_TIMESTAMP", &timestamp)
                .header("POLY_NONCE", "0")
                .send()
                .await
                .map_err(|e| anyhow!("GET /auth/derive-api-key: {}", e))?;

            if !resp2.status().is_success() {
                return Err(anyhow!(
                    "Impossible de dériver les credentials Polymarket: HTTP {}",
                    resp2.status()
                ));
            }
            resp2.json().await.map_err(|e| anyhow!("parse derive-api-key response: {}", e))?
        };

        info!("Credentials Polymarket CLOB dérivés pour {}", address_str);
        Ok(ApiCreds {
            api_key: key_resp.api_key,
            secret: key_resp.secret,
            passphrase: key_resp.passphrase,
            address: address_str,
        })
    }

    #[allow(dead_code)]
    /// POST /order avec backoff exponentiel sur les erreurs retryables (425, 429, 500, 503).
    async fn post_order_with_retry(
        &self,
        creds: &ApiCreds,
        body: String,
    ) -> Result<PostOrderResponse> {
        let http = self.http.clone();
        let creds = creds.clone();

        let policy = ExponentialBackoffBuilder::new()
            .with_initial_interval(Duration::from_millis(500))
            .with_multiplier(2.0)
            .with_max_interval(Duration::from_secs(30))
            .with_max_elapsed_time(Some(Duration::from_secs(120)))
            .build();

        retry(policy, || {
            let http = http.clone();
            let creds = creds.clone();
            let body = body.clone();
            async move {
                let timestamp = Utc::now().timestamp().to_string();
                let sig =
                    Self::compute_hmac_sig(&creds.secret, &timestamp, "POST", "/order", &body)
                        .map_err(backoff::Error::permanent)?;

                let resp = http
                    .post(format!("{}/order", CLOB_API_BASE))
                    .header("Content-Type", "application/json")
                    .header("POLY_ADDRESS", &creds.address)
                    .header("POLY_API_KEY", &creds.api_key)
                    .header("POLY_PASSPHRASE", &creds.passphrase)
                    .header("POLY_SIGNATURE", &sig)
                    .header("POLY_TIMESTAMP", &timestamp)
                    .body(body)
                    .send()
                    .await
                    .map_err(|e| backoff::Error::transient(anyhow!(e)))?;

                let status = resp.status().as_u16();
                if matches!(status, 425 | 429 | 500 | 503) {
                    warn!("Polymarket HTTP {} — retry dans quelques secondes", status);
                    return Err(backoff::Error::transient(anyhow!("HTTP {}", status)));
                }

                let body_txt = resp.text().await
                    .map_err(|e| backoff::Error::permanent(anyhow!(e)))?;
                warn!("CLOB response body: {}", body_txt);
                serde_json::from_str::<PostOrderResponse>(&body_txt)
                    .map_err(|e| backoff::Error::permanent(anyhow!(e)))
            }
        })
        .await
    }

    #[allow(dead_code)]
    async fn get_tick_size(&self, token_id: &str) -> Result<f64> {
        let resp = self
            .http
            .get(format!("{}/tick-size?token_id={}", CLOB_API_BASE, token_id))
            .send()
            .await
            .map_err(|e| anyhow!("GET /tick-size: {}", e))?;

        if !resp.status().is_success() {
            return Err(anyhow!("GET /tick-size -> HTTP {}", resp.status()));
        }

        let body: TickSizeResponse = resp
            .json()
            .await
            .map_err(|e| anyhow!("parse tick-size response: {}", e))?;
        Ok(body.minimum_tick_size)
    }

    #[allow(dead_code)]
    async fn get_fee_rate_bps(&self, token_id: &str) -> Result<u64> {
        let resp = self
            .http
            .get(format!("{}/fee-rate?token_id={}", CLOB_API_BASE, token_id))
            .send()
            .await
            .map_err(|e| anyhow!("GET /fee-rate: {}", e))?;

        if !resp.status().is_success() {
            return Err(anyhow!("GET /fee-rate -> HTTP {}", resp.status()));
        }

        let body: FeeRateResponse = resp
            .json()
            .await
            .map_err(|e| anyhow!("parse fee-rate response: {}", e))?;
        Ok(body.base_fee)
    }

    #[allow(dead_code)]
    async fn calculate_market_price(
        &self,
        token_id: &str,
        side: &str,
        amount: f64,
        order_type: &str,
    ) -> Result<f64> {
        let resp = self
            .http
            .get(format!("{}/book?token_id={}", CLOB_API_BASE, token_id))
            .send()
            .await
            .map_err(|e| anyhow!("GET /book: {}", e))?;

        if !resp.status().is_success() {
            return Err(anyhow!("GET /book -> HTTP {}", resp.status()));
        }

        let book: OrderBookResponse = resp
            .json()
            .await
            .map_err(|e| anyhow!("parse order book response: {}", e))?;

        let levels = if side == "BUY" { &book.asks } else { &book.bids };
        if levels.is_empty() {
            return Err(anyhow!("order book vide pour {}", token_id));
        }

        let mut total = 0.0;
        for level in levels.iter().rev() {
            let price = level
                .price
                .parse::<f64>()
                .map_err(|e| anyhow!("parse ask price '{}': {}", level.price, e))?;
            let size = level
                .size
                .parse::<f64>()
                .map_err(|e| anyhow!("parse ask size '{}': {}", level.size, e))?;
            total += if side == "BUY" { price * size } else { size };
            if total >= amount {
                return Ok(price);
            }
        }

        if order_type == "FOK" {
            return Err(anyhow!("pas assez de liquidité pour matcher {}", amount));
        }

        levels[0]
            .price
            .parse::<f64>()
            .map_err(|e| anyhow!("parse fallback market price: {}", e))
    }

    #[allow(dead_code)]
    fn market_buy_amounts(amount_usdc: f64, price: f64, tick_size: f64) -> Result<(u64, u64, f64)> {
        if amount_usdc < 1.0 {
            return Err(anyhow!("Mode Market requiert un montant >= 1 USDC"));
        }

        let round_config = Self::round_config(tick_size)?;
        let raw_price = Self::round_normal(price, round_config.0);
        let raw_maker_amt = Self::round_down(amount_usdc, round_config.1);
        let mut raw_taker_amt = raw_maker_amt / raw_price;
        if Self::decimal_places(raw_taker_amt) > round_config.2 {
            raw_taker_amt = Self::round_up(raw_taker_amt, round_config.2 + 4);
            if Self::decimal_places(raw_taker_amt) > round_config.2 {
                raw_taker_amt = Self::round_down(raw_taker_amt, round_config.2);
            }
        }

        Ok((
            Self::to_token_decimals(raw_maker_amt)?,
            Self::to_token_decimals(raw_taker_amt)?,
            raw_taker_amt,
        ))
    }

    #[allow(dead_code)]
    fn round_config(tick_size: f64) -> Result<(u32, u32, u32)> {
        if (tick_size - 0.1).abs() < 1e-9 {
            Ok((1, 2, 3))
        } else if (tick_size - 0.01).abs() < 1e-9 {
            Ok((2, 2, 4))
        } else if (tick_size - 0.001).abs() < 1e-9 {
            Ok((3, 2, 5))
        } else if (tick_size - 0.0001).abs() < 1e-9 {
            Ok((4, 2, 6))
        } else {
            Err(anyhow!("tick size non supporté: {}", tick_size))
        }
    }

    #[allow(dead_code)]
    fn round_down(x: f64, digits: u32) -> f64 {
        let factor = 10f64.powi(digits as i32);
        (x * factor).floor() / factor
    }

    #[allow(dead_code)]
    fn round_normal(x: f64, digits: u32) -> f64 {
        let factor = 10f64.powi(digits as i32);
        (x * factor).round() / factor
    }

    #[allow(dead_code)]
    fn round_up(x: f64, digits: u32) -> f64 {
        let factor = 10f64.powi(digits as i32);
        (x * factor).ceil() / factor
    }

    #[allow(dead_code)]
    fn decimal_places(x: f64) -> u32 {
        let s = format!("{}", x);
        s.split('.').nth(1).map(|part| part.len() as u32).unwrap_or(0)
    }

    #[allow(dead_code)]
    fn to_token_decimals(x: f64) -> Result<u64> {
        let scaled = x * 1_000_000.0;
        let rounded = scaled.round();
        if !rounded.is_finite() || rounded < 0.0 {
            return Err(anyhow!("montant invalide: {}", x));
        }
        Ok(rounded as u64)
    }

    // ── EIP-712 ───────────────────────────────────────────────────────────────

    /// Domain separator du contrat CTFExchange (Polygon mainnet, chain_id=137).
    pub fn ctf_domain_separator() -> Result<[u8; 32]> {
        let domain_typehash = keccak256(
            b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
        );
        let name_hash = keccak256(b"Polymarket CTF Exchange");
        let version_hash = keccak256(b"1");
        let contract = Address::from_str(CTF_EXCHANGE_ADDR)
            .map_err(|_| anyhow!("adresse CTFExchange invalide"))?;

        let mut buf = [0u8; 5 * 32];
        buf[0..32].copy_from_slice(domain_typehash.as_slice());
        buf[32..64].copy_from_slice(name_hash.as_slice());
        buf[64..96].copy_from_slice(version_hash.as_slice());
        buf[96..128].copy_from_slice(&U256::from(POLYGON_CHAIN_ID).to_be_bytes::<32>());
        let mut addr_pad = [0u8; 32];
        addr_pad[12..].copy_from_slice(contract.as_slice());
        buf[128..160].copy_from_slice(&addr_pad);

        Ok(*keccak256(&buf))
    }

    /// Domain separator de ClobAuthDomain (pas de verifyingContract).
    pub fn clob_auth_domain_separator() -> [u8; 32] {
        let domain_typehash =
            keccak256(b"EIP712Domain(string name,string version,uint256 chainId)");
        let name_hash = keccak256(b"ClobAuthDomain");
        let version_hash = keccak256(b"1");

        let mut buf = [0u8; 4 * 32];
        buf[0..32].copy_from_slice(domain_typehash.as_slice());
        buf[32..64].copy_from_slice(name_hash.as_slice());
        buf[64..96].copy_from_slice(version_hash.as_slice());
        buf[96..128].copy_from_slice(&U256::from(POLYGON_CHAIN_ID).to_be_bytes::<32>());

        *keccak256(&buf)
    }

    /// Hash EIP-712 complet d'un ordre CTFExchange à signer.
    pub fn order_signing_hash(
        salt: U256,
        maker: Address,
        token_id: U256,
        maker_amount: U256,
        taker_amount: U256,
        fee_rate_bps: U256,
        side: u8,
        signature_type: u8,
    ) -> Result<B256> {
        let order_typehash = keccak256(
            b"Order(uint256 salt,address maker,address signer,address taker,uint256 tokenId,\
              uint256 makerAmount,uint256 takerAmount,uint256 expiration,uint256 nonce,\
              uint256 feeRateBps,uint8 side,uint8 signatureType)",
        );

        let mut maker_pad = [0u8; 32];
        maker_pad[12..].copy_from_slice(maker.as_slice());

        // 13 champs × 32 octets = 416 octets
        let mut buf = [0u8; 13 * 32];
        buf[0..32].copy_from_slice(order_typehash.as_slice());
        buf[32..64].copy_from_slice(&salt.to_be_bytes::<32>());
        buf[64..96].copy_from_slice(&maker_pad); // maker
        buf[96..128].copy_from_slice(&maker_pad); // signer = maker (EOA)
        // taker = Address::ZERO (buf[128..160] déjà à zéro)
        buf[160..192].copy_from_slice(&token_id.to_be_bytes::<32>());
        buf[192..224].copy_from_slice(&maker_amount.to_be_bytes::<32>());
        buf[224..256].copy_from_slice(&taker_amount.to_be_bytes::<32>());
        // expiration = 0 (buf[256..288] zéro)
        // nonce = 0 (buf[288..320] zéro)
        buf[320..352].copy_from_slice(&fee_rate_bps.to_be_bytes::<32>());
        buf[383] = side; // side uint8 dans le slot [352..384], octet LSB
        buf[415] = signature_type; // signatureType uint8 dans le slot [384..416], octet LSB

        let struct_hash = keccak256(&buf);
        let domain_sep = Self::ctf_domain_separator()?;

        // "\x19\x01" || domainSeparator || structHash
        let mut final_buf = [0u8; 66];
        final_buf[0] = 0x19;
        final_buf[1] = 0x01;
        final_buf[2..34].copy_from_slice(&domain_sep);
        final_buf[34..66].copy_from_slice(struct_hash.as_slice());

        Ok(keccak256(&final_buf))
    }

    /// Hash EIP-712 du message ClobAuth à signer pour l'auth L1.
    pub fn clob_auth_signing_hash(address: Address, timestamp: &str, nonce: u64) -> Result<B256> {
        let typehash = keccak256(
            b"ClobAuth(address address,string timestamp,uint256 nonce,string message)",
        );

        let mut addr_pad = [0u8; 32];
        addr_pad[12..].copy_from_slice(address.as_slice());
        let ts_hash = keccak256(timestamp.as_bytes());
        let msg_hash = keccak256(CLOB_AUTH_MSG.as_bytes());

        // 5 champs × 32 octets
        let mut buf = [0u8; 5 * 32];
        buf[0..32].copy_from_slice(typehash.as_slice());
        buf[32..64].copy_from_slice(&addr_pad);
        buf[64..96].copy_from_slice(ts_hash.as_slice());
        buf[96..128].copy_from_slice(&U256::from(nonce).to_be_bytes::<32>());
        buf[128..160].copy_from_slice(msg_hash.as_slice());

        let struct_hash = keccak256(&buf);
        let domain_sep = Self::clob_auth_domain_separator();

        let mut final_buf = [0u8; 66];
        final_buf[0] = 0x19;
        final_buf[1] = 0x01;
        final_buf[2..34].copy_from_slice(&domain_sep);
        final_buf[34..66].copy_from_slice(struct_hash.as_slice());

        Ok(keccak256(&final_buf))
    }

    /// Sérialise une signature alloy en "0x<r><s><v>" (65 octets, v = 27 ou 28).
    fn sig_to_hex(sig: &alloy::primitives::Signature) -> String {
        let r = sig.r();
        let s = sig.s();
        let v = 27u8 + u8::from(sig.v());
        let mut bytes = [0u8; 65];
        bytes[..32].copy_from_slice(&r.to_be_bytes::<32>());
        bytes[32..64].copy_from_slice(&s.to_be_bytes::<32>());
        bytes[64] = v;
        format!("0x{}", bytes.iter().map(|b| format!("{:02x}", b)).collect::<String>())
    }

    /// Calcule la signature HMAC-SHA256 pour les headers L2.
    /// message = timestamp + method + path + body (apostrophes → guillemets)
    pub fn compute_hmac_sig(
        secret: &str,
        timestamp: &str,
        method: &str,
        path: &str,
        body: &str,
    ) -> Result<String> {
        let secret_bytes = URL_SAFE
            .decode(secret)
            .map_err(|e| anyhow!("HMAC secret decode: {}", e))?;

        let body_normalized = body.replace('\'', "\"");
        let message = format!("{}{}{}{}", timestamp, method, path, body_normalized);

        let mut mac = Hmac::<Sha256>::new_from_slice(&secret_bytes)
            .map_err(|e| anyhow!("HMAC key: {}", e))?;
        mac.update(message.as_bytes());
        let result = mac.finalize().into_bytes();

        Ok(URL_SAFE.encode(result))
    }

    /// Retourne le client SDK authentifié, le créant au premier appel puis le réutilisant.
    /// Élimine le coût de authenticate() + dérivation API key à chaque ordre (~400ms).
    /// Les caches internes du SDK (tick_size, fee_rate_bps) sont aussi conservés.
    async fn get_or_create_sdk_client(&self) -> Result<SdkClobClient<Authenticated<Normal>>> {
        let mut guard = self.sdk_client.lock().await;
        if let Some(client) = guard.as_ref() {
            return Ok(client.clone());
        }

        let sdk_signer = self
            .sdk_signer
            .as_ref()
            .ok_or_else(|| anyhow!("POLYMARKET_PRIVATE_KEY requis pour le mode Market"))?;

        let auth_builder = SdkClobClient::new(CLOB_API_BASE, SdkConfig::default())
            .map_err(|e| anyhow!("SDK client init: {}", e))?
            .authentication_builder(sdk_signer);

        let client = if let Some(funder) = self.config.polymarket_funder.as_deref() {
            let funder = Address::from_str(funder)
                .map_err(|e| anyhow!("POLYMARKET_FUNDER invalide: {}", e))?;
            let signature_type = match self.config.polymarket_signature_type.unwrap_or(2) {
                0 => SdkSignatureType::Eoa,
                1 => SdkSignatureType::Proxy,
                2 => SdkSignatureType::GnosisSafe,
                other => {
                    return Err(anyhow!(
                        "POLYMARKET_SIGNATURE_TYPE={} invalide (attendu 0, 1 ou 2)",
                        other
                    ));
                }
            };
            auth_builder
                .funder(funder)
                .signature_type(signature_type)
                .authenticate()
                .await
                .map_err(|e| anyhow!("SDK authenticate avec funder: {}", e))?
        } else {
            auth_builder
                .authenticate()
                .await
                .map_err(|e| anyhow!("SDK authenticate: {}", e))?
        };

        info!("Client SDK Polymarket authentifié et mis en cache");
        *guard = Some(client.clone());
        Ok(client)
    }

    async fn submit_market_order(
        &self,
        token_id_str: &str,
        submitted_at: DateTime<Utc>,
    ) -> Result<OrderResult> {
        use std::time::Instant;

        let sdk_signer = self
            .sdk_signer
            .as_ref()
            .ok_or_else(|| anyhow!("POLYMARKET_PRIVATE_KEY requis pour le mode Market"))?;

        let t0 = Instant::now();
        let client = self.get_or_create_sdk_client().await?;
        let sdk_client_ms = t0.elapsed().as_millis();

        let amount = Decimal::from_str(&format!("{:.6}", self.config.trade_amount_usdc))
            .map_err(|e| anyhow!("montant Decimal invalide: {}", e))?;

        // Prix plafond 0.99 : le CLOB matche au meilleur ask disponible.
        // Évite le fetch de l'order book (~200-250ms) à chaque ordre.
        let max_price = Decimal::from_str("0.99")
            .map_err(|e| anyhow!("prix max Decimal invalide: {}", e))?;

        let t1 = Instant::now();
        let order = client
            .market_order()
            .token_id(token_id_str)
            .amount(Amount::usdc(amount).map_err(|e| anyhow!("Amount::usdc: {}", e))?)
            .price(max_price)
            .side(SdkSide::Buy)
            .order_type(SdkOrderType::FOK)
            .build()
            .await
            .map_err(|e| anyhow!("SDK build market_order: {}", e))?;
        let build_ms = t1.elapsed().as_millis();

        let t2 = Instant::now();
        let signed_order = client
            .sign(sdk_signer, order)
            .await
            .map_err(|e| anyhow!("SDK sign order: {}", e))?;
        let sign_ms = t2.elapsed().as_millis();

        let t3 = Instant::now();
        let resp = client
            .post_order(signed_order)
            .await
            .map_err(|e| anyhow!("SDK post_order: {}", e))?;
        let post_ms = t3.elapsed().as_millis();
        let ack_at = Utc::now();

        info!(
            "Ordre FOK envoyé via SDK | token={} amount={}USDC | timing: sdk_client={}ms build={}ms sign={}ms post={}ms total={}ms",
            token_id_str, self.config.trade_amount_usdc,
            sdk_client_ms, build_ms, sign_ms, post_ms, t0.elapsed().as_millis()
        );

        Ok(OrderResult {
            order_id: format!("{:?}", resp.order_id).trim_matches('"').to_string(),
            status: format!("{:?}", resp.status).trim_matches('"').to_string(),
            submitted_at,
            ack_at,
        })
    }

    async fn submit_market_order_with_retry(
        &self,
        token_id_str: &str,
        submitted_at: DateTime<Utc>,
    ) -> Result<OrderResult> {
        let mut attempt = 0usize;

        loop {
            match self.submit_market_order(token_id_str, submitted_at).await {
                Ok(result) => return Ok(result),
                Err(e) if Self::is_fok_unfilled_error(&e) && attempt < FOK_RETRY_DELAYS_SECS.len() => {
                    let delay_secs = FOK_RETRY_DELAYS_SECS[attempt];
                    warn!(
                        "Ordre FOK non rempli immédiatement pour token={} — retry {}/{} dans {}s",
                        token_id_str,
                        attempt + 1,
                        FOK_RETRY_DELAYS_SECS.len(),
                        delay_secs
                    );
                    tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                    attempt += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }

    pub(crate) fn is_fok_unfilled_error(err: &anyhow::Error) -> bool {
        let msg = err.to_string().to_ascii_lowercase();
        msg.contains("fok orders are fully filled or killed")
            || msg.contains("order couldn't be fully filled")
    }
}
