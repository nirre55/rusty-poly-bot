#!/usr/bin/env bash
# Start all 4 ensemble strategies, each in its own terminal tab/window.
# Falls back to background processes if no supported terminal is found.

ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"

CONFIGS=(
    "configs/btc_ensemble.env"
    "configs/eth_ensemble.env"
    "configs/btc_15m_ensemble.env"
    "configs/eth_15m_ensemble.env"
)

run_in_terminal() {
    local cfg="$1"
    local title="${cfg//configs\//}"
    title="${title//.env/}"
    local cmd="cd '$ROOT' && STRATEGY_CONFIG='$cfg' cargo run; echo 'Press Enter to close...'; read"

    if command -v gnome-terminal &>/dev/null; then
        gnome-terminal --title="$title" -- bash -c "$cmd"
    elif command -v xterm &>/dev/null; then
        xterm -title "$title" -e bash -c "$cmd" &
    elif command -v konsole &>/dev/null; then
        konsole --new-tab -p tabtitle="$title" -e bash -c "$cmd" &
    else
        # Fallback: background process, output to logs dir
        local log="logs/${title}.out"
        mkdir -p logs
        STRATEGY_CONFIG="$cfg" cargo run >"$log" 2>&1 &
        echo "[$title] running in background → $log (PID $!)"
    fi
}

for cfg in "${CONFIGS[@]}"; do
    run_in_terminal "$cfg"
    sleep 0.5
done

echo "Started ${#CONFIGS[@]} strategies."
