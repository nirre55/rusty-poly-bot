use rusty_poly_bot::logger::{TradeLogger, TradeRecord};
use std::fs;

fn make_record(trade_id: &str, prediction: &str) -> TradeRecord {
    TradeRecord {
        trade_id: trade_id.to_string(),
        symbol: "BTCUSDT".to_string(),
        interval: "5m".to_string(),
        signal_close_time_utc: "2024-01-01T00:00:00+00:00".to_string(),
        target_candle_open_time_utc: "2024-01-01T00:05:00+00:00".to_string(),
        prediction: prediction.to_string(),
        entry_side: "BUY".to_string(),
        entry_order_type: "DRY_RUN".to_string(),
        order_status: "DRY_RUN".to_string(),
        signal_to_submit_start_ms: 10,
        submit_start_to_ack_ms: 5,
        signal_to_ack_ms: 15,
        trade_open_to_order_ack_ms: 20,
        outcome: "PENDING".to_string(),
    }
}

fn tmp_dir(label: &str) -> std::path::PathBuf {
    // Dossier unique par test pour éviter les conflits entre tests parallèles
    let dir = std::env::temp_dir()
        .join(format!("rusty_poly_bot_test_{}_{}", label, uuid::Uuid::new_v4()));
    dir
}

// --- Création du CSV ---

#[test]
fn test_logger_creates_csv_file() {
    let dir = tmp_dir("creates_csv");
    let _logger = TradeLogger::new(dir.to_str().unwrap()).unwrap();
    assert!(dir.join("trades.csv").exists(), "Le fichier trades.csv doit être créé");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_logger_csv_contains_headers() {
    let dir = tmp_dir("headers");
    let _logger = TradeLogger::new(dir.to_str().unwrap()).unwrap();
    let content = fs::read_to_string(dir.join("trades.csv")).unwrap();

    for header in &[
        "trade_id", "symbol", "interval", "prediction",
        "order_status", "outcome", "signal_to_ack_ms",
    ] {
        assert!(content.contains(header), "Header manquant: {}", header);
    }
    fs::remove_dir_all(&dir).ok();
}

/// P7 : si le fichier existe mais est vide (crash pendant init), les headers doivent être écrits
#[test]
fn test_logger_writes_headers_on_empty_existing_file() {
    let dir = tmp_dir("empty_file");
    fs::create_dir_all(&dir).unwrap();
    let csv_path = dir.join("trades.csv");

    // Créer un fichier vide (simule un crash pendant l'initialisation précédente)
    fs::write(&csv_path, "").unwrap();
    assert_eq!(fs::metadata(&csv_path).unwrap().len(), 0);

    let _logger = TradeLogger::new(dir.to_str().unwrap()).unwrap();
    let content = fs::read_to_string(&csv_path).unwrap();
    assert!(content.contains("trade_id"), "Les headers doivent être écrits sur un fichier vide");
    fs::remove_dir_all(&dir).ok();
}

/// Si le CSV existe déjà avec des données, le logger ne doit pas réécrire les headers
#[test]
fn test_logger_does_not_overwrite_existing_data() {
    let dir = tmp_dir("no_overwrite");
    let logger = TradeLogger::new(dir.to_str().unwrap()).unwrap();

    logger.log_trade(&make_record("id-first", "UP")).unwrap();
    let content_before = fs::read_to_string(dir.join("trades.csv")).unwrap();
    let line_count_before = content_before.lines().count();

    // Recréer le logger sur le même dossier
    let logger2 = TradeLogger::new(dir.to_str().unwrap()).unwrap();
    logger2.log_trade(&make_record("id-second", "DOWN")).unwrap();

    let content_after = fs::read_to_string(dir.join("trades.csv")).unwrap();
    let line_count_after = content_after.lines().count();

    assert_eq!(line_count_after, line_count_before + 1, "Une seule ligne doit être ajoutée");
    assert!(!content_after.contains("trade_id\ntrade_id"), "Les headers ne doivent pas être dupliqués");
    fs::remove_dir_all(&dir).ok();
}

// --- log_trade ---

#[test]
fn test_log_trade_appends_record() {
    let dir = tmp_dir("append");
    let logger = TradeLogger::new(dir.to_str().unwrap()).unwrap();

    logger.log_trade(&make_record("test-id-123", "UP")).unwrap();

    let content = fs::read_to_string(dir.join("trades.csv")).unwrap();
    assert!(content.contains("test-id-123"));
    assert!(content.contains("BTCUSDT"));
    assert!(content.contains("UP"));
    assert!(content.contains("PENDING"));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_log_trade_multiple_records_all_present() {
    let dir = tmp_dir("multiple");
    let logger = TradeLogger::new(dir.to_str().unwrap()).unwrap();

    logger.log_trade(&make_record("id-001", "UP")).unwrap();
    logger.log_trade(&make_record("id-002", "DOWN")).unwrap();
    logger.log_trade(&make_record("id-003", "UP")).unwrap();

    let content = fs::read_to_string(dir.join("trades.csv")).unwrap();
    assert!(content.contains("id-001"));
    assert!(content.contains("id-002"));
    assert!(content.contains("id-003"));

    // 1 header + 3 records = 4 lignes
    assert_eq!(content.lines().count(), 4);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_log_trade_latency_fields_written() {
    let dir = tmp_dir("latency");
    let logger = TradeLogger::new(dir.to_str().unwrap()).unwrap();

    let mut record = make_record("lat-test", "UP");
    record.signal_to_submit_start_ms = 42;
    record.submit_start_to_ack_ms = 17;
    record.signal_to_ack_ms = 59;
    record.trade_open_to_order_ack_ms = 310;

    logger.log_trade(&record).unwrap();

    let content = fs::read_to_string(dir.join("trades.csv")).unwrap();
    assert!(content.contains("42"));
    assert!(content.contains("17"));
    assert!(content.contains("59"));
    assert!(content.contains("310"));
    fs::remove_dir_all(&dir).ok();
}
