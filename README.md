# Shogi Codex

Command-line shogi engine and UI scaffolded according to the specification in `AGENTS.md`.

## Project layout

```
src/
├── cli/             # Text-based UI helpers (input handling, board rendering, app runner)
├── engine/          # Core representation: bitboards, moves, search stubs
├── game/            # Game orchestration, players, and time control
├── lib.rs           # Exposes CLI + engine modules
└── main.rs          # Entry point invoking the CLI app
```

### Engine modules
- `engine::state` enumerates sides, piece kinds, and `Square` helpers.
- `engine::bitboard`, `engine::board`, and `engine::hand` store the position using 128-bit bitboards and packed hands.
- `engine::movement` describes move primitives.
- `engine::search` contains the alpha-beta skeleton, evaluator, and CPU strength definitions.

### Game modules
- `game::config`, `game::player`, and `game::controller` describe modes (Player vs CPU, CPU vs CPU), player roles, and the high-level match state machine.
- `game::timer` implements the 10 minute + 30 second byoyomi clocks.

### CLI
- `cli::app` collects user input for mode/strength, boots up the controller, and prints a simplified board diagram using `cli::board_render`.

## Usage

```bash
cargo run
```

The CLI will prompt for:
1. Game mode (`Player vs CPU` or `CPU vs CPU`).
2. CPU strength (`Weak`, `Normal`, or `Strong`, implemented via search depth presets).

During play:
- Moves use `<from><to>[+]` with digits for both file and rank (e.g. `7776`, `2888+`). Legacy rank letters (`7g7f`) are still accepted for convenience.
- Drops use `<piece>*<square>` like `P*57` (or `P*5e`). Piece codes: `P, L, N, S, G, B, R, K`.
- The board shows row indices `1-9` and the current pieces in hand for each side so you can see available drops at a glance.
- Type `moves` to list legal moves, `help` for the quick reference, `resign` to resign on your turn, or `/resign` to force an immediate resignation for Sente at any time.
- Clocks follow `10:00 + 30s` byoyomi; once main time is gone each move must complete within 30 seconds.
- A new debug mode (prompted at startup) prints CPU thinking summaries—best move, evaluation score, depth, and node count—and streams detailed search traces and played moves to `debug.log` (per-branch logging plus move history).

## Next steps

- Hook search thinking time into the time manager (iterative deepening or pondering) so CPU play obeys byoyomi automatically.
- Improve evaluation/scoring heuristics beyond pure material and consider positional bonuses.
- Persist move history or add a notation log/export format for post-game review.
- Add niceties such as move hints, undo/redo for analysis, or configurable time controls.
