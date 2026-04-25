// =============================================================================
// Search Strength — Confidence-Based Termination
// =============================================================================
//
// Instead of capping the search at a fixed depth, the engine searches as deep
// as time allows and decides to stop early when it is "confident enough" that
// the current best move won't change with deeper search.
//
// Confidence is measured by two signals:
//   1. **Move stability** — how many consecutive iterations returned the same
//      best move.  A move that survives multiple depth increases is very likely
//      correct.
//   2. **Score stability** — how much the evaluation changed between the last
//      two completed iterations.  A small change means the position is well
//      understood; a large swing means something tactical is still unresolved.
//
// The strength level controls how quickly the engine becomes "confident":
//
//   Weak:   stops after 2 stable iterations above depth 4 (decisive, quick).
//   Normal: stops after 3 stable iterations above depth 6 (balanced).
//   Strong: stops after 4 stable iterations above depth 8 (thorough).
//
// This design has several advantages over a hard depth cap:
//   - In simple positions, even Strong stops early (obvious best move).
//   - In complex positions, Weak still searches deep enough to avoid blunders.
//   - The engine naturally adapts its thinking time to position difficulty.
//   - No artificial ceiling prevents the engine from finding deep tactics.
//
// The hard upper bound (MAX_DEPTH = 64) exists only as a safety net; in
// practice, time always runs out before reaching it.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(Default)]
pub enum SearchStrength {
    Weak,
    #[default]
    Normal,
    Strong,
}

/// Parameters that control when the engine decides to stop searching.
pub struct ConfidenceParams {
    /// Don't consider early termination before this depth.
    pub min_depth: u8,
    /// Number of consecutive iterations with the same best move required
    /// to declare confidence.
    pub stable_iterations: u8,
    /// Maximum score change between iterations to be considered "stable".
    /// Larger = more lenient (stops sooner).
    pub score_threshold: i32,
}

/// Safety net: never iterate beyond this depth regardless of time.
pub const MAX_DEPTH: u8 = 64;

impl SearchStrength {
    /// Returns the confidence parameters for this strength level.
    pub fn confidence(self) -> ConfidenceParams {
        match self {
            SearchStrength::Weak => ConfidenceParams {
                min_depth: 4,
                stable_iterations: 2,
                score_threshold: 80,
            },
            SearchStrength::Normal => ConfidenceParams {
                min_depth: 6,
                stable_iterations: 3,
                score_threshold: 40,
            },
            SearchStrength::Strong => ConfidenceParams {
                min_depth: 12,
                stable_iterations: 5,
                score_threshold: 10,
            },
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            SearchStrength::Weak => "Weak",
            SearchStrength::Normal => "Normal",
            SearchStrength::Strong => "Strong",
        }
    }
}
