# rusty-poly-bot

Bot de trading live en Rust pour les marchés Polymarket BTC Up/Down 5m, piloté par un signal généré depuis Binance.

## Stratégie implémentée

**`three_candle_rsi7_reversal`**

- Source : WebSocket Binance `BTCUSDT` kline 5m (bougies fermées uniquement)
- Signal : 3 bougies consécutives de même couleur + condition RSI7
  - Série rouge + RSI7 ≤ 35 → prédiction `UP` → acheter `UP` sur Polymarket
  - Série verte + RSI7 ≥ 65 → prédiction `DOWN` → acheter `DOWN` sur Polymarket

## Architecture

```
src/
├── main.rs                          # Boucle principale
├── config.rs                        # Chargement .env
├── binance.rs                       # WebSocket Binance kline
├── strategy.rs                      # Trait Strategy + Signal
├── strategies/
│   ├── mod.rs
│   └── three_candle_rsi7_reversal.rs
├── polymarket.rs                    # Client Polymarket (stub V2)
└── logger.rs                        # Logs console + CSV trades
logs/
    trades.csv                       # Généré automatiquement
```

## Installation

### 1. Installer Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# Windows : https://rustup.rs
rustup update stable
```

### 2. Cloner / préparer le projet

```bash
cd rusty-poly-bot
```

### 3. Configurer l'environnement

```bash
cp .env.example .env
# Editer .env selon vos besoins
```

## Lancer en dry-run (V1 — aucune clé requise)

```bash
EXECUTION_MODE=dry-run cargo run
```

Le bot se connecte au WebSocket Binance, calcule les signaux en temps réel et simule les ordres sans aucun appel réseau Polymarket.

## Lancer en mode réel (V2 — ordres Polymarket)

> **Note** : Le mode `market` et `limit` sont des stubs en V1.
> L'implémentation V2 nécessite une clé privée EVM et la signature EIP-712.

```bash
# Remplir POLYMARKET_API_KEY et POLYMARKET_API_SECRET dans .env
EXECUTION_MODE=market cargo run
```

## Lancer les tests unitaires

```bash
cargo test
```

Tests couverts :
- Couleur d'une bougie (verte/rouge/doji)
- Détection de 3 bougies consécutives de même couleur
- Calcul du RSI7 (cas limite, only gains, pas assez de données)
- Absence de signal sans condition RSI valide
- Mapping prédiction `UP`/`DOWN`
- Construction du slug Polymarket

## Lire les logs de latence

Chaque trade est enregistré dans `logs/trades.csv` avec les colonnes :

| Colonne | Description |
|---|---|
| `signal_to_submit_start_ms` | Délai entre réception du signal et début soumission |
| `submit_start_to_ack_ms` | Délai entre soumission et accusé réception |
| `signal_to_ack_ms` | Latence totale signal → ack |
| `trade_open_to_order_ack_ms` | Délai depuis clôture de bougie jusqu'à ack |

```bash
# Exemple : lire les dernières lignes
tail -n 20 logs/trades.csv
```

Les logs console sont structurés avec les préfixes :
- `[BOUGIE FERMÉE]` — chaque bougie 5m fermée
- `[SIGNAL]` — signal détecté par la stratégie
- `[ORDRE ENVOYÉ]` — envoi de l'ordre
- `[ORDRE ACK]` — accusé réception + latence

## Roadmap

| Version | Contenu |
|---|---|
| **V1** (actuelle) | WebSocket Binance, stratégie, dry-run, logs console + CSV |
| **V2** | Ordres réels Polymarket (market + limit), mesures de latence |
| **V3** | User WebSocket Polymarket, suivi des fills, limit → market |

## Ajouter une nouvelle stratégie

1. Créer `src/strategies/ma_strategie.rs` en implémentant le trait `Strategy` :

```rust
use crate::strategy::{Signal, Strategy};
use crate::binance::Candle;

pub struct MaStrategie;

impl Strategy for MaStrategie {
    fn name(&self) -> &str { "ma_strategie" }
    fn on_closed_candle(&mut self, candle: &Candle) -> Option<Signal> {
        // logique ici
        None
    }
}
```

2. L'enregistrer dans `src/strategies/mod.rs`
3. L'activer dans `main.rs` : `Box::new(MaStrategie::new())`

## Hypothèses API Polymarket (V2)

- Résolution slug → tokenIds : `GET https://gamma-api.polymarket.com/markets?slug={slug}`
- Placement d'ordre : `POST https://clob.polymarket.com/order` avec payload signé EIP-712
- Le format exact du slug `btc-updown-5m-<YYYYMMDD>` doit être validé contre l'API live
- Les tokenIds UP/DOWN varient par expiration de marché
