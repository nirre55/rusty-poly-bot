//! Test de latence low-latency path.
//!
//! Lance avec :
//!   CONFIRM_LIVE_ORDER=yes POLYMARKET_SLUG_PREFIX=doge-updown-5m cargo run --bin test_latency
//!
//! Ce binaire :
//!  1. Résout le marché doge-updown-5m courant
//!  2. Warm caches SDK (tick_size, fee_rate, neg_risk)
//!  3. Pré-signe les ordres UP + DOWN
//!  4. POST direct l'ordre UP via le fast path
//!  5. Affiche les timings détaillés de chaque étape

use anyhow::Result;
use chrono::Utc;
use rusty_poly_bot::config::{Config, ExecutionMode};
use rusty_poly_bot::polymarket::PolymarketClient;
use rusty_poly_bot::strategy::{Prediction, Signal};
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    if std::env::var("CONFIRM_LIVE_ORDER").as_deref() != Ok("yes") {
        eprintln!(
            "[ABORT] Définir CONFIRM_LIVE_ORDER=yes pour autoriser l'ouverture d'un ordre réel."
        );
        std::process::exit(1);
    }

    let mut config = Config::from_env()?;
    if matches!(config.execution_mode, ExecutionMode::DryRun) {
        eprintln!("[WARN] .env en dry-run — passage forcé en mode Market.");
        config.execution_mode = ExecutionMode::Market;
    }
    config.trade_amount_usdc = 1.0;

    let slug_prefix = config.polymarket_slug_prefix.clone();
    let client = PolymarketClient::new(config);

    // ── 1. Warm-up connexion ──────────────────────────────────────────────────
    println!("\n[1/5] Warm-up connexion CLOB...");
    let t = Instant::now();
    client.warm_up().await;
    println!("      ✓ warm_up = {}ms", t.elapsed().as_millis());

    // ── 2. Resolve market ─────────────────────────────────────────────────────
    let now_ms = Utc::now().timestamp_millis();
    let interval_ms = 5 * 60 * 1000i64;
    let next_open_ms = (now_ms / interval_ms + 1) * interval_ms;
    let slug = PolymarketClient::build_slug(&slug_prefix, next_open_ms);

    println!("[2/5] Résolution marché : slug={}", slug);
    let t = Instant::now();
    let market = client.resolve_market(&slug).await?;
    println!("      ✓ resolve_market = {}ms", t.elapsed().as_millis());
    println!("      ✓ UP  token={}", market.up_token_id);
    println!("      ✓ DOWN token={}", market.down_token_id);

    // ── 3. Warm SDK caches ────────────────────────────────────────────────────
    println!("[3/5] Warm caches SDK (tick_size, fee_rate, neg_risk)...");
    let t = Instant::now();
    client.warm_sdk_caches(&market).await;
    println!("      ✓ warm_sdk_caches = {}ms", t.elapsed().as_millis());

    // ── 4. Pre-sign orders ────────────────────────────────────────────────────
    println!("[4/5] Pré-signature ordres UP + DOWN (1 USDC)...");
    let t = Instant::now();
    client.pre_sign_orders(&market, 1.0).await;
    println!("      ✓ pre_sign_orders = {}ms", t.elapsed().as_millis());

    // ── 5. Fast POST (mesure latence réelle) ──────────────────────────────────
    let signal = Signal {
        prediction: Prediction::Up,
        signal_candle_close_time: Utc::now(),
        rsi: 30.0,
        strategy_name: "test_latency".to_string(),
    };

    println!("[5/5] Placement ordre UP via FAST POST (1 USDC)...");
    let t = Instant::now();
    let result = client.place_order(&signal, &market, 1.0).await;
    let total_ms = t.elapsed().as_millis();

    match result {
        Ok(order) => {
            println!("      ✓ order_id={}", order.order_id);
            println!("      ✓ status={}", order.status);
            println!("      ✓ place_order total = {}ms", total_ms);
        }
        Err(e) => {
            eprintln!("      ✗ Erreur: {}", e);
            eprintln!("      ✗ total = {}ms", total_ms);
        }
    }

    // ── Résumé ────────────────────────────────────────────────────────────────
    println!("\n══════════════════════════════════════");
    println!(" FAST POST latence = {}ms", total_ms);
    println!("══════════════════════════════════════\n");

    // ── 6. Comparaison : SDK normal (sans pré-signature) ──────────────────────
    println!("[BONUS] Comparaison : SDK normal (build+sign+post)...");
    let signal2 = Signal {
        prediction: Prediction::Down,
        signal_candle_close_time: Utc::now(),
        rsi: 70.0,
        strategy_name: "test_latency".to_string(),
    };
    let t = Instant::now();
    let result2 = client.place_order(&signal2, &market, 1.0).await;
    let sdk_ms = t.elapsed().as_millis();

    match result2 {
        Ok(order) => {
            println!("      ✓ order_id={}", order.order_id);
            println!("      ✓ status={}", order.status);
            println!("      ✓ SDK normal total = {}ms", sdk_ms);
        }
        Err(e) => {
            eprintln!("      ✗ Erreur: {}", e);
            eprintln!("      ✗ total = {}ms", sdk_ms);
        }
    }

    println!("\n══════════════════════════════════════");
    println!(" FAST POST  = {}ms", total_ms);
    println!(" SDK normal = {}ms", sdk_ms);
    println!(" Différence = {}ms", sdk_ms as i128 - total_ms as i128);
    println!("══════════════════════════════════════\n");

    Ok(())
}
