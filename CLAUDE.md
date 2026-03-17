# rusty-poly-bot — Contexte de travail

## Règle importante pour Claude

**A chaque fin de session ou après toute modification significative du projet, mettre à jour ce fichier CLAUDE.md :**
- Déplacer les tâches terminées dans "Ce qui est fait"
- Retirer les tâches terminées de "Ce qui reste à faire"
- Ajouter tout nouveau point d'attention ou décision technique prise
- Mettre à jour l'état actuel en haut du fichier

## Etat actuel : V1 terminée et fonctionnelle

Le projet compile sans erreurs ni warnings. Les 7 tests unitaires passent.

```bash
cargo run     # lance le bot en dry-run
cargo test    # 7/7 tests passent
```

## Ce qui est fait (V1)

- WebSocket Binance BTCUSDT kline 5m avec reconnexion automatique
- Stratégie `three_candle_rsi7_reversal` : 3 bougies + RSI7
- Trait `Strategy` extensible pour brancher de nouvelles stratégies
- Mode dry-run fonctionnel (aucune clé requise)
- Logger CSV `logs/trades.csv` + logs console structurés
- Mesures de latence complètes (4 deltas)
- 7 tests unitaires couvrant les briques critiques

## Ce qui reste à faire

### V2 — Ordres réels Polymarket (priorité suivante)
1. **`src/polymarket.rs`** — implémenter `resolve_market()` :
   - `GET https://gamma-api.polymarket.com/markets?slug={slug}`
   - Parser `condition_id`, `tokens[].token_id` (UP/DOWN)
   - Valider le format exact du slug `btc-updown-5m-<YYYYMMDD>`

2. **`src/polymarket.rs`** — implémenter `place_order()` en mode réel :
   - Signature EIP-712 des ordres Polymarket CLOB
   - `POST https://clob.polymarket.com/order`
   - Nécessite clé privée EVM dans `.env`
   - Ref : https://docs.polymarket.com/#place-order

3. **Mode `limit`** : prix limit à définir (mid-price du marché ?)
4. **Gestion des erreurs réseau** : retry avec backoff sur les appels Polymarket

### V3 — Suivi des positions
- User WebSocket Polymarket pour suivre les fills
- Mode `limit → market après N secondes` si non rempli
- Mise à jour du champ `outcome` dans le CSV après résolution du marché

### Améliorations optionnelles
- RSI lissé (Wilder/EMA) au lieu du RSI simple actuel
- Ajouter d'autres stratégies dans `src/strategies/` (EMA, filtres horaires)
- Tests d'intégration avec mock WebSocket

## Architecture clé

```
src/
├── main.rs                          # boucle principale, ne pas sur-complexifier
├── strategy.rs                      # trait Strategy — NE PAS modifier l'interface
├── strategies/
│   └── three_candle_rsi7_reversal.rs  # stratégie active
└── polymarket.rs                    # stubs V2 à implémenter ici
```

## Points d'attention

- Le slug Polymarket `btc-updown-5m-<YYYYMMDD>` est une hypothèse — à valider live
- Les tokenIds UP/DOWN changent par expiration de marché
- RSI7 actuel = RSI simple (non lissé) — suffisant pour V1
- `CANDLE_HISTORY = 11` (RSI_PERIOD+1+3) — ne pas réduire

## Commandes utiles

```bash
cargo run                          # dry-run
cargo test                         # tests unitaires
RUST_LOG=debug cargo run           # logs verbeux
cargo check                        # vérification rapide sans compiler
```
