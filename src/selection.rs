//! A/B halving sequence + small-number for locating a card in a physical pile.
//!
//! Given pile size N and target index i (0..N), produce:
//! - A sequence of A (top half) / B (bottom half) until remainder ≤ 10.
//! - A final random in 2..=10 (or 2..=size) for the last step.

use rand::Rng;

/// Result of the A/B selection: the halving sequence and the final small number.
#[derive(Debug, Clone)]
pub struct AbInstruction {
    /// e.g. "A", "B", "A", "A", "A", "B", "A", "A"
    pub sequence: Vec<Half>,
    /// Final step: number in 2..=10 (or 2..=remaining size).
    pub final_number: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Half {
    A, // top half
    B, // bottom half
}

impl std::fmt::Display for Half {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Half::A => write!(f, "A"),
            Half::B => write!(f, "B"),
        }
    }
}

impl AbInstruction {
    /// Format sequence as "A, B, A, A, ..." and append final number.
    pub fn display_string(&self) -> String {
        let seq: String = self
            .sequence
            .iter()
            .map(|h| h.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        format!("{} — {}", seq, self.final_number)
    }
}

/// Generate instructions to reach a random card in a pile of size `pile_size`.
/// `pile_size` is the current estimated count (must be >= 1).
/// Returns (instruction, index_used) where index_used is in 0..pile_size.
pub fn generate_ab_instruction(pile_size: u32, rng: &mut impl Rng) -> (AbInstruction, u32) {
    assert!(pile_size >= 1, "pile must have at least one card");
    let index = rng.gen_range(0..pile_size);
    let instruction = ab_instruction_for_index(pile_size, index, rng);
    (instruction, index)
}

/// Build A/B instruction for a specific index (0..pile_size). Used for testing
/// and when we've already chosen the index (e.g. from weighted draw).
pub fn ab_instruction_for_index(pile_size: u32, index: u32, rng: &mut impl Rng) -> AbInstruction {
    let mut sequence = Vec::new();
    let mut start = 0u32;
    let mut len = pile_size;

    while len > 10 {
        let half = len / 2; // approximate; we don't need perfect split
        let top_end = start + half;
        if index < top_end {
            sequence.push(Half::A);
            len = half;
            // start unchanged
        } else {
            sequence.push(Half::B);
            start = top_end;
            len = len - half;
        }
    }

    // Final step: 2..=min(10, len)
    let max_final = len.min(10).max(2);
    let final_number = rng.gen_range(2..=max_final);

    AbInstruction {
        sequence,
        final_number,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn ab_sequence_reduces_pile() {
        let mut rng = StdRng::seed_from_u64(42);
        let (inst, _idx) = generate_ab_instruction(1384, &mut rng);
        assert!(!inst.sequence.is_empty());
        assert!(inst.final_number >= 2 && inst.final_number <= 10);
    }
}
