//! Data models for piles, settings, and pack generation.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Kind of pile: determines how we pick from it and how it's used in packs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PileType {
    /// Single pool; pure random across all cards (user keeps all trainers under one type).
    Trainers,
    /// Energy of a specific type; even likelihood per type, then pick one from this pile.
    Energy { energy_type: String },
    /// Bulk; multiple piles weighted by size; pick pile then position via A/B.
    Bulk,
    /// Value / rarity proxy; optional price range maps to rarity via lookup table.
    Value {
        #[serde(default)]
        price_min_usd: Option<f64>,
        #[serde(default)]
        price_max_usd: Option<f64>,
        /// Explicit rarity override (optional); if set, used instead of price→rarity.
        #[serde(default)]
        rarity: Option<Rarity>,
    },
}

/// Rarity used for slot filling and value-pile matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    DoubleRare,
    UltraRare,
}

/// A single pile of cards (trainers, energy, bulk, or value).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pile {
    pub id: Uuid,
    pub name: String,
    #[serde(rename = "type")]
    pub pile_type: PileType,
    /// Estimated count; decremented when we pick from this pile.
    pub estimated_count: u32,
}

impl Pile {
    pub fn new(name: String, pile_type: PileType, estimated_count: u32) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            pile_type,
            estimated_count,
        }
    }
}

/// Pack type: determines layout and odds. Only Modern is implemented; others are stubbed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PackTypeId {
    #[default]
    Modern,
    /// Stub for future implementation.
    Classic,
    /// Stub for future implementation.
    Legacy,
}

impl PackTypeId {
    pub fn label(self) -> &'static str {
        match self {
            PackTypeId::Modern => "Modern",
            PackTypeId::Classic => "Classic (coming soon)",
            PackTypeId::Legacy => "Legacy (coming soon)",
        }
    }

    pub fn is_implemented(self) -> bool {
        matches!(self, PackTypeId::Modern)
    }
}

/// Global app settings (persisted).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Cards per pack (default 5).
    pub pack_size: u32,
    /// Pack type (Modern implemented; others stubbed).
    pub pack_type: PackTypeId,
    /// Include an energy slot when generating packs (default false).
    pub add_energy_to_packs: bool,
    /// Energy type IDs to treat as "out" (excluded from energy draw).
    #[serde(default)]
    pub energy_types_out: Vec<String>,
    /// Card recognition via camera: show scan button and store recognized cards (default false).
    #[serde(default)]
    pub image_rec_enabled: bool,
    /// URL of local recognition service (e.g. http://127.0.0.1:5001). Empty = not configured.
    #[serde(default)]
    pub image_rec_service_url: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            pack_size: 5,
            pack_type: PackTypeId::Modern,
            add_energy_to_packs: false,
            energy_types_out: Vec::new(),
            image_rec_enabled: false,
            image_rec_service_url: None,
        }
    }
}

// --------------- Pack history (for viewing past packs and recognized cards) ---------------

/// One slot in a saved pack: A/B instruction plus optional recognized card data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotHistoryEntry {
    pub slot_number: u32,
    pub slot_role: String,
    pub pile_name: String,
    pub instruction_display: String,
    /// Pokemon TCG API card id (e.g. swsh12-123) from recognition; None if not scanned.
    #[serde(default)]
    pub recognized_card_id: Option<String>,
    /// Display name (from API or user override).
    #[serde(default)]
    pub card_name: Option<String>,
    /// Holo / reverse holo (user can edit).
    #[serde(default)]
    pub card_holo: Option<bool>,
    /// Card image URL (from API or after regenerate).
    #[serde(default)]
    pub card_image_url: Option<String>,
}

/// A saved pack: created when user opens a pack; can be viewed later and shows A/B or card details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackHistoryEntry {
    pub id: Uuid,
    pub created_at: String,
    pub slots: Vec<SlotHistoryEntry>,
    #[serde(default)]
    pub warning: Option<String>,
}

/// Critical low threshold: below this, pile is considered too small for reliable A/B drawing.
pub const CRITICAL_LOW_THRESHOLD: u32 = 40;
