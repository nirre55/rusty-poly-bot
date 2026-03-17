use rusty_poly_bot::config::ExecutionMode;

#[test]
fn test_execution_mode_as_str_dryrun() {
    assert_eq!(ExecutionMode::DryRun.as_str(), "DRY_RUN");
}

#[test]
fn test_execution_mode_as_str_market() {
    assert_eq!(ExecutionMode::Market.as_str(), "MARKET");
}

#[test]
fn test_execution_mode_as_str_limit() {
    assert_eq!(ExecutionMode::Limit.as_str(), "LIMIT");
}

/// Vérifie que les valeurs as_str() sont cohérentes avec les status d'ordre Polymarket
#[test]
fn test_execution_mode_as_str_uppercase_consistent() {
    for mode in [ExecutionMode::DryRun, ExecutionMode::Market, ExecutionMode::Limit] {
        let s = mode.as_str();
        assert_eq!(s, s.to_uppercase(), "as_str() doit être en MAJUSCULES: {}", s);
    }
}

/// Vérifie que Config::from_env() fonctionne avec les valeurs par défaut (aucun .env requis)
#[test]
fn test_config_from_env_defaults() {
    // Supprimer les vars pour tester les defaults
    std::env::remove_var("EXECUTION_MODE");
    std::env::remove_var("TRADE_AMOUNT_USDC");
    std::env::remove_var("SYMBOL");
    std::env::remove_var("INTERVAL");

    let config = rusty_poly_bot::config::Config::from_env().unwrap();
    assert_eq!(config.symbol, "btcusdt");
    assert_eq!(config.interval, "5m");
    assert_eq!(config.trade_amount_usdc, 10.0);
    assert!(matches!(config.execution_mode, ExecutionMode::DryRun));
}

/// Vérifie que TRADE_AMOUNT_USDC=0 est rejeté et remplacé par la valeur par défaut
#[test]
fn test_config_trade_amount_zero_uses_default() {
    std::env::set_var("TRADE_AMOUNT_USDC", "0");
    let config = rusty_poly_bot::config::Config::from_env().unwrap();
    assert_eq!(config.trade_amount_usdc, 10.0);
    std::env::remove_var("TRADE_AMOUNT_USDC");
}

/// Vérifie que TRADE_AMOUNT_USDC négatif est rejeté
#[test]
fn test_config_trade_amount_negative_uses_default() {
    std::env::set_var("TRADE_AMOUNT_USDC", "-50");
    let config = rusty_poly_bot::config::Config::from_env().unwrap();
    assert_eq!(config.trade_amount_usdc, 10.0);
    std::env::remove_var("TRADE_AMOUNT_USDC");
}

/// Vérifie que TRADE_AMOUNT_USDC non-parseable est rejeté
#[test]
fn test_config_trade_amount_invalid_string_uses_default() {
    std::env::set_var("TRADE_AMOUNT_USDC", "abc");
    let config = rusty_poly_bot::config::Config::from_env().unwrap();
    assert_eq!(config.trade_amount_usdc, 10.0);
    std::env::remove_var("TRADE_AMOUNT_USDC");
}

/// Vérifie qu'un EXECUTION_MODE inconnu (faute de frappe) bascule sur DryRun
#[test]
fn test_config_unknown_execution_mode_defaults_to_dryrun() {
    std::env::set_var("EXECUTION_MODE", "MARKET"); // majuscules incorrectes
    let config = rusty_poly_bot::config::Config::from_env().unwrap();
    assert!(matches!(config.execution_mode, ExecutionMode::DryRun));
    std::env::remove_var("EXECUTION_MODE");
}

/// Vérifie que Debug ne contient pas les secrets en clair
#[test]
fn test_config_debug_redacts_secrets() {
    std::env::set_var("POLYMARKET_API_KEY", "super_secret_key");
    std::env::set_var("POLYMARKET_API_SECRET", "super_secret_value");
    let config = rusty_poly_bot::config::Config::from_env().unwrap();
    let debug_str = format!("{:?}", config);
    assert!(!debug_str.contains("super_secret_key"), "La clé API ne doit pas apparaître dans Debug");
    assert!(!debug_str.contains("super_secret_value"), "Le secret API ne doit pas apparaître dans Debug");
    assert!(debug_str.contains("[REDACTED]"), "Les secrets doivent être remplacés par [REDACTED]");
    std::env::remove_var("POLYMARKET_API_KEY");
    std::env::remove_var("POLYMARKET_API_SECRET");
}
