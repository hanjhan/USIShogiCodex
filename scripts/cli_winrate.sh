#!/usr/bin/env bash
# Runs `cargo run --bin cli` N times, accepting the default answer for every
# interactive prompt (which selects CPU vs CPU with default strengths / times),
# captures each game's output, and reports the Sente / Gote / Draw counts.
#
# Usage: ./scripts/cli_winrate.sh [N] [log_dir]
#   N:       number of games to play (default 10)
#   log_dir: where per-game outputs are written (default ./cli-winrate-logs)

set -euo pipefail

N="${1:-10}"
LOG_DIR="${2:-./cli-winrate-logs}"
mkdir -p "$LOG_DIR"

# Build once so per-game timing isn't dominated by compile.
cargo build --release --bin cli >/dev/null

sente=0
gote=0
draw=0
other=0

for i in $(seq 1 "$N"); do
    log="$LOG_DIR/game_$(printf '%03d' "$i").log"
    # `yes ''` feeds an infinite stream of empty lines; every prompt sees
    # an empty reply and falls back to its default value.  Stdin closes
    # when the binary exits.
    yes '' | cargo run --release --quiet --bin cli >"$log" 2>&1 || true

    line=$(grep -m1 '^Game over:' "$log" || echo 'Game over: unknown')
    case "$line" in
        *"Sente wins"*) sente=$((sente + 1)); tag='Sente' ;;
        *"Gote wins"*)  gote=$((gote + 1));   tag='Gote'  ;;
        *"draw"*)       draw=$((draw + 1));   tag='Draw'  ;;
        *)              other=$((other + 1)); tag='?'     ;;
    esac
    printf '[%03d/%s] %-5s  %s\n' "$i" "$N" "$tag" "$line"
done

total=$((sente + gote + draw + other))
echo
echo "=== Results over $total games ==="
echo "  Sente wins: $sente"
echo "  Gote  wins: $gote"
echo "  Draws:      $draw"
if [[ $other -gt 0 ]]; then
    echo "  Unparsed:   $other"
fi

if [[ $total -gt 0 ]]; then
    awk -v s="$sente" -v g="$gote" -v d="$draw" -v t="$total" 'BEGIN {
        printf "  Sente rate: %.1f%%\n", 100*s/t
        printf "  Gote  rate: %.1f%%\n", 100*g/t
        printf "  Draw  rate: %.1f%%\n", 100*d/t
    }'
fi
