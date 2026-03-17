Tu dois creer un projet Rust autonome, dans un dossier totalement separe de tout autre code existant.

Objectif :
Construire un bot live Rust pour trader les marches Polymarket BTC Up/Down 5m a partir d'un signal genere depuis Binance.

Contrainte importante :
- Il ne faut supposer aucune dependance a un repo Python existant.
- Le projet doit vivre seul dans son propre dossier.
- Le prompt doit etre executable meme si on part d'un dossier vide.

Strategie a implementer :
- Marche observe : `BTCUSDT`
- Timeframe : `5m`
- Source des bougies : Binance websocket `kline_5m`
- On regarde uniquement les bougies fermees
- Si les `3` dernieres bougies fermees sont de la meme couleur :
  - serie rouge : verifier `RSI7 <= 35`
  - serie verte : verifier `RSI7 >= 65`
- Si la condition est valide, prendre l'inverse sur la bougie suivante :
  - serie rouge + RSI valide => prediction `VERTE`
  - serie verte + RSI valide => prediction `ROUGE`
- Sur Polymarket :
  - prediction `VERTE` => acheter `UP`
  - prediction `ROUGE` => acheter `DOWN`

But principal :
- Avoir une implementation Rust optimisee pour la latence
- Mesurer proprement la latence d'execution
- Produire un bot robuste, simple, modulaire et exploitable en live
- Prevoir une architecture qui permette d'ajouter facilement de nouvelles strategies plus tard

Structure attendue du projet :
- `Cargo.toml`
- `src/main.rs`
- `src/config.rs`
- `src/binance.rs`
- `src/polymarket.rs`
- `src/strategy.rs`
- `src/strategies/`
- `src/logger.rs`
- `README.md`
- `.env.example`
- `logs/`

Fonctionnalites a developper en priorite :
1. Initialiser un projet Rust propre
2. Charger la configuration depuis `.env`
3. Se connecter au websocket Binance pour recevoir les bougies `5m`
4. Maintenir un historique suffisant pour calculer `RSI7`
5. Implementer la logique de strategie
6. Construire le slug Polymarket du type `btc-updown-5m-<timestamp>`
7. Resoudre le marche cible et les token ids `UP` / `DOWN`
8. Envoyer un ordre Polymarket
9. Logger les latences d'execution
10. Sauvegarder les trades dans un CSV

Extensibilite strategie :
- Ne pas coder la logique de strategie en dur dans `main.rs`
- Prevoir une abstraction claire pour brancher plusieurs strategies
- Exemple possible :
  - un trait `Strategy`
  - un dossier `src/strategies/`
  - une structure de resultat commune pour toutes les strategies
- Le moteur principal doit pouvoir appeler une strategie active sans etre reecrit a chaque nouvelle idee
- La premiere strategie a implementer est :
  - `three_candle_rsi7_reversal`
- Mais l'architecture doit permettre d'ajouter ensuite facilement :
  - EMA
  - RSI mean reversion
  - previous candle color
  - filtres horaires
  - combinaisons d'indicateurs

Modes d'execution souhaites :
- `dry-run`
- `market`
- `limit`
- plus tard si possible :
  - `limit -> market apres N secondes`
  - suivi d'ordres via user websocket Polymarket

Mesures de latence a enregistrer :
- `signal_computed_at_utc`
- `order_submit_started_at_utc`
- `order_ack_at_utc`
- `signal_to_submit_start_ms`
- `submit_start_to_ack_ms`
- `signal_to_ack_ms`
- `trade_open_to_order_ack_ms`

Logs console souhaites :
- cloture de bougie
- signal detecte
- ordre envoye
- ack recu
- statut ordre
- erreurs reseau / execution

Format CSV minimal des trades :
- `trade_id`
- `symbol`
- `interval`
- `signal_close_time_utc`
- `target_candle_open_time_utc`
- `prediction`
- `entry_side`
- `entry_order_type`
- `order_status`
- `signal_to_submit_start_ms`
- `submit_start_to_ack_ms`
- `signal_to_ack_ms`
- `trade_open_to_order_ack_ms`
- `outcome`

Contraintes d'implementation :
- Code Rust propre et modulaire
- Bon handling des erreurs
- Limiter les allocations et traitements inutiles sur le chemin critique
- Eviter les appels reseau inutiles au moment de l'entree
- Preparer les composants reseau et clients le plus tot possible
- Ajouter des tests unitaires pour les briques critiques

Tests unitaires attendus au minimum :
- calcul de la couleur d'une bougie
- detection de `3` bougies consecutives de meme couleur
- calcul du `RSI7`
- detection du signal final de strategie
- construction du slug Polymarket
- mapping prediction `VERTE/ROUGE` vers `UP/DOWN`
- parsing des reponses ou evenements critiques si une couche API Polymarket est abstraite
- tests sur l'abstraction strategie si plusieurs strategies peuvent etre branchees

Plan de livraison recommande :
- V1 :
  - websocket Binance
  - strategie
  - dry-run
  - logs console + CSV
- V2 :
  - ordres reels Polymarket
  - mode `market`
  - mode `limit`
  - mesures de latence
- V3 :
  - user websocket Polymarket
  - suivi des fills
  - `limit -> market`

Documentation a produire :
- comment installer Rust
- comment configurer le `.env`
- comment lancer en dry-run
- comment lancer en reel
- comment lancer les tests unitaires
- comment lire les logs de latence

Important :
- Si certaines parties Polymarket ne peuvent pas etre finalisees immediatement, construire quand meme une architecture propre avec stubs clairs
- Documenter explicitement les hypotheses API
- Favoriser une premiere version executable plutot qu'une architecture trop ambitieuse
- Garder un design suffisamment simple, mais ne pas bloquer l'ajout futur de nouvelles strategies

Livrable attendu :
- Un dossier Rust autonome, compilable si possible
- Une base de code propre et modulaire
- Un `README.md` complet
- Un bot minimal fonctionnel ou, si necessaire, un prototype propre pret a etre complete
