# CLAUDE.md — Project Context for Claude Code

## Project Overview

Shogi (Japanese chess) engine in Rust with three frontends:
- **CLI** (`cargo run --bin cli`) — interactive terminal game
- **USI** (`cargo build --release --bin usi`) — protocol engine for external GUI apps
- **Think** (`cargo run --bin think`) — interactive position analyser (startpos or SFEN file), streams `info` lines indefinitely, supports `undo` / move input to navigate

## Build & Test

```bash
cargo build --release            # build all binaries
cargo test --release --lib       # run unit tests (7 tests)
cargo clippy --release           # lint check (should be 0 warnings)
cargo run --release --bin cli    # play in terminal
cargo run --release --bin usi    # run USI engine
cargo run --release --bin think  # analyse positions interactively
```

## Architecture

- `src/engine/` — pure game logic, no I/O
  - `board.rs` — Board state with incremental Zobrist hashing + incremental eval scores
  - `movegen.rs` — legal move generation with pin-based optimization
  - `movelist.rs` — stack-allocated MoveList (no heap alloc per node)
  - `search/alpha_beta.rs` — main search engine (iterative deepening + alpha-beta)
  - `search/evaluator.rs` — king safety + king threat evaluation (material is incremental)
  - `eval_tables.rs` — pre-computed material + PST lookup tables
  - `zobrist.rs` — Zobrist hash key tables (static, initialized once)
- `src/game/` — game orchestration, time control, player config
  - `controller.rs` — central coordinator between engine and frontends
- `src/cli/` — terminal UI
- `src/usi/` — USI protocol handler
- `src/think/` — thinking-mode analyser
  - `sfen.rs` — SFEN parser (`<board> <side> <hand> <move_no>`)
  - `command.rs` — command / move-notation parser (CLI + USI accepted)
  - `session.rs` — owns board + move stack + persistent searcher; starts/stops background searches
  - `mod.rs` — main interactive loop: prompt board source → search/input loop

## Key Design Decisions

- **Incremental evaluation**: material + PST + pawn advancement are maintained in `Board.eval_score[side]`, updated on every make/undo. The evaluator only adds king safety/threat on top.
- **Make/unmake**: `board.make_move()` returns `UndoInfo`, `board.undo_move()` restores state. Used in search and legality checking.
- **Pin optimization**: `compute_pinned()` casts rays from the king to find pinned pieces. Non-pinned, non-king moves skip the expensive `is_in_check()` call.
- **Confidence-based termination**: instead of a fixed depth cap, the engine stops when the best move is stable across iterations. Strength level controls confidence threshold.
- **Stack-allocated MoveList**: avoids heap allocation for move generation at every node.

## Search Techniques

- Iterative deepening with aspiration windows
- PVS (Principal Variation Search)
- Null-move pruning (adaptive R = 3 + depth/6)
- Late Move Reductions (logarithmic: ln(depth) × ln(moveIndex) / 1.4)
- Late Move Pruning at depth 1-6
- Futility pruning (depth 1-6) and reverse futility pruning (depth 1-7)
- Delta pruning in quiescence
- Check extensions
- TT (8M entries), killer moves, history heuristic
- Dedicated capture-only generator for quiescence (`loud_moves`)
- Instant stop on mate detection + aspiration bypass for mate scores
- USI `info` output with PV, depth, score, nodes, nps, hashfull

## What Was Done (Session History)

### Session 1 (earlier, summarized)
- Implemented Zobrist hashing (incremental XOR)
- Rewrote `is_square_attacked` with reverse attack tables (43× speedup)
- Fixed engine resigning with valid moves (fallback move seeding)
- Fixed byoyomi time-loss (200ms safety margin)
- Raised MAX_SEARCH_TIME to 600s
- Full codebase refactoring (28 clippy warnings fixed)
- Implemented futility pruning, delta pruning, aspiration windows
- Created `scripts/cli_winrate.sh` for CPU-vs-CPU testing

### Session 3 (current)
- **Thinking mode (`cargo run --bin think`)**: interactive position analyser
  - Startpos or custom SFEN file (first non-blank, non-`#` line) as starting position
  - Engine searches indefinitely (no time budget), confidence stop disabled via Strong strength in practice; aborted by any user input
  - Streams human-readable info per iteration: `depth N | eval ±X | Y nodes (Z/s) | pv: ...`
  - Commands: move (CLI `7776` or USI `7g7f` both accepted), `undo`/`u`, `moves`, `help`, `quit`/`exit`
  - Move stack via `Vec<Move>` replayed from startpos on undo (O(n), trivial) — designed to upgrade to a tree later
  - Persistent `AlphaBetaSearcher` across restarts preserves TT / killer / history
- **`InfoOutputMode` enum** replaces bool `usi_output`: variants `None | Usi | Think`. `set_usi_output(bool)` wrapper kept for the USI binary

### Session 2 (earlier)
- **Make/unmake in search**: replaced `board.clone()` with `make_move/undo_move` in alpha_beta, quiescence, and root search
- **Stack-allocated MoveList**: replaced `Vec<Move>` with fixed-capacity array (no heap alloc per node)
- **Incremental evaluation**: pre-computed `EvalTable` (material + PST + pawn advancement), Board maintains `eval_score[side]`, evaluator only adds king safety
- **Capture-only quiescence generator**: `MoveGenerator::loud_moves()` skips quiet moves and drops entirely — 2.5× NPS improvement
- **Pin-based legality optimization**: `compute_pinned()` detects pinned pieces; non-pinned moves skip is_in_check — 1.44× NPS improvement
- **Make/unmake in legality check**: replaced clone in `push_move` with make/undo
- **King threat evaluation**: attacker proximity with non-linear scaling table + drop threat amplifier
- **Confidence-based search termination**: replaced fixed depth cap with move/score stability detection; strength controls confidence threshold
- **Instant mate stop**: when a mate score is found, stop iterating immediately; bypass aspiration window re-search on mate
- **USI time management rewrite**: budget = remaining/40 + byoyomi, capped at remaining/3
- **USI `info` output**: PV tracking through triangular PV table, outputs depth/score/time/nodes/nps/hashfull/pv after each iteration
- **Aggressive pruning overhaul**: logarithmic LMR (divisor 1.4), Late Move Pruning, adaptive NMP (R=3+depth/6), extended futility/reverse futility to depth 6-7, 8M TT
- **Drop piece formatting**: USI drop moves use uppercase piece letter
- **README rewrite**: documented CLI and USI modes, engine features, project layout

### Performance Progression
- Starting point: ~11k nps (before is_in_check rewrite)
- After reverse attack tables: ~490k nps
- After MoveList + incremental eval: ~670k nps
- After loud_moves quiescence: ~1.65M nps
- After pin optimization: ~2.3M nps
- After aggressive pruning: depth 18 in 60s from opening (was depth 12)
