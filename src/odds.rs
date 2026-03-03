//! Pack layouts and odds: slot order, rarity rolls, and price→rarity lookup.
//!
//! Modern 5-card layout is derived from Scarlet & Violet–style packs (rare in last slot;
//! rest common/uncommon). Other pack types are stubbed for future use.

use crate::models::{PackTypeId, Rarity};
use rand::Rng;

/// Role of a slot in the physical pack (determines which slot is "rare" etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotRole {
    Common,
    Uncommon,
    Rare,
    Energy,
    Trainer,
}

/// Per-slot config: role and odds for what rarity we actually put there.
#[derive(Debug, Clone)]
pub struct SlotOdds {
    pub role: SlotRole,
    /// Cumulative odds for outcome: (Rarity, weight). We roll and pick one.
    /// e.g. [(Common, 90), (Uncommon, 10)] => 90% common, 10% uncommon.
    pub rarity_weights: Vec<(Rarity, u32)>,
}

/// Full layout for a pack type: number of slots and odds per slot index (0-based).
pub struct PackLayout {
    pub slot_count: u32,
    /// Slot index -> odds. Length must match slot_count.
    pub slots: Vec<SlotOdds>,
}

impl PackLayout {
    /// Returns the layout for the given pack type. When `add_energy` is true, one slot
    /// (slot 2 in physical order) is an energy slot. Stubbed types return None.
    pub fn for_pack_type(id: PackTypeId, add_energy: bool) -> Option<PackLayout> {
        match id {
            PackTypeId::Modern => Some(modern_5_card_layout(add_energy)),
            PackTypeId::Classic | PackTypeId::Legacy => None,
        }
    }
}

/// Modern 5-card: physical order matches SV-style — slot 5 (index 4) is the rare slot.
/// When add_energy is true, slot 2 (index 1) is an energy slot.
fn modern_5_card_layout(add_energy: bool) -> PackLayout {
    let slot2 = if add_energy {
        SlotOdds {
            role: SlotRole::Energy,
            rarity_weights: vec![(Rarity::Common, 1)], // unused for energy
        }
    } else {
        SlotOdds {
            role: SlotRole::Common,
            rarity_weights: vec![(Rarity::Common, 90), (Rarity::Uncommon, 10)],
        }
    };
    PackLayout {
        slot_count: 5,
        slots: vec![
            // Slot 1: common-heavy
            SlotOdds {
                role: SlotRole::Common,
                rarity_weights: vec![(Rarity::Common, 90), (Rarity::Uncommon, 10)],
            },
            // Slot 2: energy (if enabled) or common-heavy
            slot2,
            // Slot 3: trainer (pure random from trainers pile)
            SlotOdds {
                role: SlotRole::Trainer,
                rarity_weights: vec![(Rarity::Common, 1)],
            },
            // Slot 4: uncommon-heavy
            SlotOdds {
                role: SlotRole::Uncommon,
                rarity_weights: vec![(Rarity::Common, 25), (Rarity::Uncommon, 75)],
            },
            // Slot 5: rare slot — ~1/7 double rare, ~1/15 ultra rare, else rare (simplified from SV)
            SlotOdds {
                role: SlotRole::Rare,
                rarity_weights: vec![
                    (Rarity::Rare, 800),
                    (Rarity::DoubleRare, 140),
                    (Rarity::UltraRare, 60),
                ],
            },
        ],
    }
}

/// Roll a rarity for a slot given its odds. Returns the chosen rarity.
pub fn roll_rarity_for_slot(slot_odds: &SlotOdds, rng: &mut impl Rng) -> Rarity {
    let total: u32 = slot_odds.rarity_weights.iter().map(|(_, w)| w).sum();
    let mut roll = rng.gen_range(0..total);
    for (rarity, weight) in &slot_odds.rarity_weights {
        if roll < *weight {
            return *rarity;
        }
        roll -= weight;
    }
    slot_odds
        .rarity_weights
        .last()
        .map(|(r, _)| *r)
        .unwrap_or(Rarity::Common)
}

// --------------- Price → Rarity (baked-in) ---------------

/// Baked-in tiers: (min_usd, max_usd) -> approximate rarity for value piles.
/// Based on rough competitive/market tiers; can be tuned via config later.
fn price_rarity_tiers() -> Vec<(f64, f64, Rarity)> {
    vec![
        (0.0, 0.25, Rarity::Common),
        (0.25, 0.50, Rarity::Common),
        (0.50, 1.0, Rarity::Uncommon),
        (1.0, 2.0, Rarity::Uncommon),
        (2.0, 4.0, Rarity::Rare),
        (4.0, 8.0, Rarity::Rare),
        (8.0, 15.0, Rarity::DoubleRare),
        (15.0, 50.0, Rarity::DoubleRare),
        (50.0, f64::MAX, Rarity::UltraRare),
    ]
}

/// Map a value pile's price range to a single "effective" rarity for matching.
/// Uses the midpoint of the range (or the tier that contains it). If no range given, treat as Rare.
pub fn price_range_to_rarity(price_min: Option<f64>, price_max: Option<f64>) -> Rarity {
    let mid = match (price_min, price_max) {
        (Some(l), Some(h)) => (l + h) / 2.0,
        (Some(l), None) => l,
        (None, Some(h)) => h,
        (None, None) => return Rarity::Rare,
    };
    let tiers = price_rarity_tiers();
    for (_l, _h, r) in tiers {
        if mid >= _l && mid < _h {
            return r;
        }
    }
    Rarity::Rare
}

/// Check whether a value pile's effective rarity is at least as high as the target
/// (so we can pick from it when we roll e.g. DoubleRare).
pub fn rarity_at_least(pile_rarity: Rarity, target: Rarity) -> bool {
    fn rank(r: Rarity) -> u8 {
        match r {
            Rarity::Common => 0,
            Rarity::Uncommon => 1,
            Rarity::Rare => 2,
            Rarity::DoubleRare => 3,
            Rarity::UltraRare => 4,
        }
    }
    rank(pile_rarity) >= rank(target)
}
