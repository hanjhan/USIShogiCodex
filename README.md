# Shogi Codex

A shogi (Japanese chess) engine written in Rust with three modes: an interactive CLI for playing games directly in the terminal, a USI protocol engine for use with external shogi GUI applications, and a "thinking mode" analyser for exploring positions.

## Modes

### CLI Mode — Play in Terminal

```bash
cargo run --bin cli
```

The CLI prompts for game settings and runs an interactive game in the terminal:

- **Game mode**: Player vs CPU or CPU vs CPU
- **Time control**: Main time + byoyomi per side
- **CPU think time**: Seconds per move
- **CPU strength**: Weak, Normal, or Strong (controls how deeply the engine searches)
- **Debug mode**: Shows CPU thinking (best move, score, depth, node count)

During play:
- Moves: `7776`, `2888+` (promotion). Legacy rank letters (`7g7f`) also accepted.
- Drops: `P*57` or `P*5e`. Piece codes: P, L, N, S, G, B, R, K.
- Commands: `moves` (list legal moves), `help`, `resign`.

### USI Mode — External GUI

Build the USI engine binary:

```bash
cargo build --release --bin usi
```

The compiled binary is at `target/release/usi`. Register it in your shogi GUI app (e.g. Shogidokoro, ShogiGUI, Electron Shogi) as an external engine. The engine communicates via the USI (Universal Shogi Interface) protocol over stdin/stdout.

Supported USI commands: `usi`, `isready`, `usinewgame`, `position`, `go` (with `btime`/`wtime`/`byoyomi`/`movetime`), `stop`, `quit`.

The engine outputs `info` lines during search showing depth, score, nodes, NPS, and the principal variation (PV), which GUI apps display as the engine's thinking.

### Thinking Mode — Analyse Positions

```bash
cargo run --release --bin think
```

Interactive position analyser. On startup, choose between the standard opening or a custom position loaded from an SFEN file (one SFEN line per file, `#` comments accepted).

Once a position is loaded the engine analyses indefinitely, printing a line per completed iteration:

```
depth 12 | eval   +42 | 1.2M nodes (1.8M/s) | pv: 7776 3334 2838+ ...
```

At any time, type one of:

- A move in CLI (`7776`, `2888+`, `P*57`) or USI (`7g7f`, `2b8h+`, `P*5e`) notation — the move is applied and analysis restarts on the new position.
- `undo` (or `u`) to take back the most recent move.
- `moves` to list legal moves in the current position.
- `help` / `?` for the command list.
- `quit` or `exit` to end the session.

Attempting to `undo` at the starting position prints an error and leaves the program running.

## Engine Features

### Search
- Iterative deepening with aspiration windows
- Alpha-beta (negamax, fail-soft) with PVS (Principal Variation Search)
- Transposition table (8M entries, Zobrist hashing)
- Null-move pruning (adaptive R = 3 + depth/6)
- Late Move Reductions (logarithmic formula)
- Late Move Pruning at shallow depths
- Futility pruning and reverse futility pruning (depth 1-7)
- Delta pruning in quiescence search
- Check extensions
- Killer moves and history heuristic for move ordering
- Confidence-based search termination (stops when the best move is stable)
- Instant stop on forced mate detection

### Evaluation
- Incremental material + piece-square table scoring (O(1) per node)
- King safety: defender proximity bonus, exposed-king penalty
- King threat: attacker proximity with non-linear scaling, drop threat amplifier
- Pre-computed evaluation lookup tables

### Performance
- ~1.5M nodes/sec on desktop hardware
- Stack-allocated move lists (no heap allocation per node)
- Make/unmake instead of board cloning
- Pin-based legality optimization (skips is_in_check for ~80% of moves)
- Dedicated capture-only generator for quiescence search
- Fast reverse-attack-table check detection

## Project Layout

```
src/
├── cli/             # Terminal UI (input, board rendering, game loop)
├── engine/          # Core engine
│   ├── bitboard.rs  # 128-bit bitboard type
│   ├── board.rs     # Board state with incremental Zobrist + eval
│   ├── eval_tables.rs # Pre-computed piece-square tables
│   ├── hand.rs      # Packed captured-piece representation
│   ├── movegen.rs   # Legal move generation with pin optimization
│   ├── movelist.rs  # Stack-allocated move list
│   ├── movement.rs  # Move struct
│   ├── search/      # Alpha-beta searcher, evaluator, strength config
│   ├── state.rs     # PlayerSide, PieceKind, Square types
│   └── zobrist.rs   # Zobrist hash key tables
├── game/            # Game orchestration, time control, player config
├── usi/             # USI protocol handler
├── think/           # Thinking-mode analyser (SFEN parser, session, commands)
├── lib.rs           # Module exports
└── main.rs          # Default entry point
```
