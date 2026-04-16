// Engine module — all pure game logic with no I/O.
//
// Sub-modules:
//   bitboard  — 128-bit bitboard type and square iterator
//   state     — core types: PlayerSide, PieceKind, Piece, Square
//   hand      — packed u32 representation of captured pieces in hand
//   movement  — Move struct (fully specified half-move)
//   board     — Board (bitboards + hands + side to move) and PositionSignature
//   movegen   — legal/pseudo-legal move generation for all piece types
//   search    — alpha-beta searcher, static evaluator, and strength config

pub mod bitboard;
pub mod board;
pub mod eval_tables;
pub mod hand;
pub mod movegen;
pub mod movelist;
pub mod movement;
pub mod search;
pub mod state;
pub mod zobrist;
