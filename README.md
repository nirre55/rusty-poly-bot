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
├── polymarket.rs                    # Client Polymarket CLOB + EIP-712
├── tracker.rs                       # Suivi des ordres ouverts + validation Binance
├── money.rs                         # Gestion Martingale (taille de position dynamique)
└── logger.rs                        # Logs console + CSV trades
logs/
├── trades.csv                       # Historique des trades
├── pending_orders.json              # Ordres en cours de suivi
└── money_state.json                 # État Martingale (losses consécutifs)
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

## Variables d'environnement

| Variable | Description | Défaut |
|---|---|---|
| `EXECUTION_MODE` | `dry-run`, `market` ou `limit` | `dry-run` |
| `SYMBOL` | Paire Binance (ex: `btcusdt`, `ethusdt`) | `btcusdt` |
| `INTERVAL` | Intervalle des bougies (ex: `5m`, `15m`, `1h`) | `5m` |
| `TRADE_AMOUNT_USDC` | Montant de base par trade en USDC | `10.0` |
| `STRATEGY` | Nom de la stratégie | `three_candle_rsi7_reversal` |
| `RSI_OVERBOUGHT` | Seuil RSI haut (signal DOWN) | `65.0` |
| `RSI_OVERSOLD` | Seuil RSI bas (signal UP) | `35.0` |
| `POLYMARKET_SLUG_PREFIX` | Préfixe slug marché (ex: `btc-updown-5m`) | `btc-updown-5m` |
| `MARTINGALE_MULTIPLIER` | Multiplicateur après chaque loss (`1.0` = désactivé) | `1.0` |
| `MARTINGALE_MAX_AMOUNT` | Plafond Martingale en USDC (`0.0` = pas de plafond) | `0.0` |
| `POLYMARKET_PRIVATE_KEY` | Clé privée EVM (hex). Requis pour mode `market` | — |
| `POLYMARKET_FUNDER` | Adresse funder si différente de l'EOA | — |
| `POLYMARKET_SIGNATURE_TYPE` | `0`=EOA, `1`=POLY_PROXY, `2`=GNOSIS_SAFE | — |
| `LOGS_DIR` | Répertoire des logs | `logs` |

## Lancer en dry-run

```bash
EXECUTION_MODE=dry-run cargo run
```

Le bot se connecte au WebSocket Binance, calcule les signaux en temps réel et simule les ordres sans aucun appel réseau Polymarket.

## Lancer en mode réel

```bash
# Remplir POLYMARKET_PRIVATE_KEY dans .env
EXECUTION_MODE=market cargo run
```

## Gestion Martingale

Taille de position dynamique : après chaque loss, le montant est multiplié par `MARTINGALE_MULTIPLIER`. Après un win, retour au montant de base.

Exemple avec `TRADE_AMOUNT_USDC=1` et `MARTINGALE_MULTIPLIER=1.3` :

| Trade | Résultat | Montant suivant |
|---|---|---|
| 1 | — | 1.00$ |
| 2 | LOSS | 1.30$ |
| 3 | LOSS | 1.69$ |
| 4 | LOSS | 2.20$ |
| 5 | WIN | 1.00$ (reset) |

L'état persiste dans `logs/money_state.json` — un restart ne reset pas la séquence.

Pour plafonner : `MARTINGALE_MAX_AMOUNT=10` limite le montant à 10 USDC max.

## Plusieurs instances (multi-actifs)

Lancer plusieurs instances en parallèle avec des `LOGS_DIR` séparés :

**PowerShell :**

```powershell
# Terminal 1 — BTC
$env:SYMBOL="btcusdt"; $env:POLYMARKET_SLUG_PREFIX="btc-updown-5m"; $env:LOGS_DIR="logs/btc"; cargo run

# Terminal 2 — ETH
$env:SYMBOL="ethusdt"; $env:POLYMARKET_SLUG_PREFIX="eth-updown-5m"; $env:LOGS_DIR="logs/eth"; cargo run
```

**Bash :**

```bash
# Terminal 1 — BTC
SYMBOL=btcusdt POLYMARKET_SLUG_PREFIX=btc-updown-5m LOGS_DIR=logs/btc cargo run

# Terminal 2 — ETH
SYMBOL=ethusdt POLYMARKET_SLUG_PREFIX=eth-updown-5m LOGS_DIR=logs/eth cargo run
```

Chaque instance a son propre tracker, money manager et logs.

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
3. L'ajouter dans `create_strategy()` de `main.rs`
4. Activer via : `STRATEGY=ma_strategie cargo run`

## API Polymarket

- Résolution slug → tokenIds : `GET https://gamma-api.polymarket.com/markets?slug={slug}`
- Placement d'ordre : `POST https://clob.polymarket.com/order` avec payload signé EIP-712
- Format slug : `{prefix}-{unix_seconds}` (ex: `btc-updown-5m-1710000000`)
- Les tokenIds UP/DOWN varient par marché (chaque bougie = un marché distinct)
- Ordres FOK (Fill or Kill) à prix plafond 0.99 pour exécution instantanée
