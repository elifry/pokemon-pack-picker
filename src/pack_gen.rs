//! Pack generation: roll slots, pick piles, produce A/B instructions, decrement counts.

use crate::models::{Pile, PileType, Rarity};
use crate::odds::{
    price_range_to_rarity, rarity_at_least, roll_rarity_for_slot, PackLayout, SlotRole,
};
use crate::selection::{ab_instruction_for_index, generate_ab_instruction, AbInstruction};
use crate::state::PersistedState;
use rand::Rng;
use std::collections::HashMap;
use uuid::Uuid;

/// One card slot in the generated pack: which pile to use and the A/B instruction.
#[derive(Debug, Clone)]
pub struct SlotInstruction {
    /// 1-based physical slot number (order to fill).
    pub slot_number: u32,
    /// Human-readable slot role (e.g. "Common", "Rare", "Energy").
    pub slot_role: String,
    pub pile_id: Uuid,
    pub pile_name: String,
    pub instruction: AbInstruction,
}

/// Result of generating one pack: instructions per slot and any warning (e.g. critical low).
#[derive(Debug, Clone)]
pub struct PackResult {
    pub slots: Vec<SlotInstruction>,
    pub warning: Option<String>,
}

/// Generate one pack: update state in place (decrement counts) and return instructions.
pub fn generate_pack(state: &mut PersistedState, rng: &mut impl Rng) -> Result<PackResult, String> {
    let layout =
        PackLayout::for_pack_type(state.settings.pack_type, state.settings.add_energy_to_packs)
            .ok_or_else(|| "Pack type not implemented".to_string())?;

    if layout.slot_count as usize != layout.slots.len() {
        return Err("Layout slot count mismatch".to_string());
    }

    let mut slots_out = Vec::with_capacity(layout.slots.len());
    let mut decrements: HashMap<Uuid, u32> = HashMap::new(); // pile_id -> count to subtract

    for (idx, slot_odds) in layout.slots.iter().enumerate() {
        let slot_number = (idx + 1) as u32;
        let slot_role_str = slot_role_label(slot_odds.role);

        match slot_odds.role {
            SlotRole::Energy => {
                let (pile, inst) = pick_energy(state, rng)?;
                decrements.entry(pile.id).or_insert(0);
                *decrements.get_mut(&pile.id).unwrap() += 1;
                slots_out.push(SlotInstruction {
                    slot_number,
                    slot_role: slot_role_str.to_string(),
                    pile_id: pile.id,
                    pile_name: pile.name.clone(),
                    instruction: inst,
                });
            }
            SlotRole::Trainer => {
                let (pile, inst) = pick_trainer(state, rng)?;
                decrements.entry(pile.id).or_insert(0);
                *decrements.get_mut(&pile.id).unwrap() += 1;
                slots_out.push(SlotInstruction {
                    slot_number,
                    slot_role: slot_role_str.to_string(),
                    pile_id: pile.id,
                    pile_name: pile.name.clone(),
                    instruction: inst,
                });
            }
            SlotRole::Common | SlotRole::Uncommon | SlotRole::Rare => {
                let rarity = roll_rarity_for_slot(slot_odds, rng);
                let (pile, inst) = pick_card_slot(state, rarity, rng)?;
                decrements.entry(pile.id).or_insert(0);
                *decrements.get_mut(&pile.id).unwrap() += 1;
                slots_out.push(SlotInstruction {
                    slot_number,
                    slot_role: slot_role_str.to_string(),
                    pile_id: pile.id,
                    pile_name: pile.name.clone(),
                    instruction: inst,
                });
            }
        }
    }

    // Apply decrements
    for (id, delta) in decrements {
        if let Some(p) = state.piles.iter_mut().find(|p| p.id == id) {
            p.estimated_count = p.estimated_count.saturating_sub(delta);
        }
    }

    let warning = critical_low_warning(state);
    Ok(PackResult {
        slots: slots_out,
        warning,
    })
}

fn slot_role_label(role: SlotRole) -> &'static str {
    match role {
        SlotRole::Common => "Common",
        SlotRole::Uncommon => "Uncommon",
        SlotRole::Rare => "Rare",
        SlotRole::Energy => "Energy",
        SlotRole::Trainer => "Trainer",
    }
}

/// Pick from trainers: single pool, pure random.
fn pick_trainer(
    state: &PersistedState,
    rng: &mut impl Rng,
) -> Result<(Pile, AbInstruction), String> {
    let trainers: Vec<&Pile> = state
        .piles
        .iter()
        .filter(|p| matches!(p.pile_type, PileType::Trainers) && p.estimated_count > 0)
        .collect();
    if trainers.is_empty() {
        return Err("No trainers pile available".to_string());
    }
    let pile = trainers[rng.gen_range(0..trainers.len())].clone();
    let (inst, _) = generate_ab_instruction(pile.estimated_count, rng);
    Ok((pile, inst))
}

/// Pick from energy: even likelihood per type (excluding "out"), then one card from that type's pile.
fn pick_energy(
    state: &PersistedState,
    rng: &mut impl Rng,
) -> Result<(Pile, AbInstruction), String> {
    let energy_piles: Vec<&Pile> = state
        .piles
        .iter()
        .filter(|p| {
            if let PileType::Energy { energy_type } = &p.pile_type {
                !state.settings.energy_types_out.contains(energy_type) && p.estimated_count > 0
            } else {
                false
            }
        })
        .collect();
    if energy_piles.is_empty() {
        return Err("No energy piles available (or all marked out)".to_string());
    }
    let pile = energy_piles[rng.gen_range(0..energy_piles.len())].clone();
    if pile.estimated_count == 0 {
        return Err("Selected energy pile has no cards".to_string());
    }
    let (inst, _) = generate_ab_instruction(pile.estimated_count, rng);
    Ok((pile, inst))
}

/// Pick a card for a non-energy slot: prefer value pile when rarity matches and we have one; else bulk.
fn pick_card_slot(
    state: &PersistedState,
    target_rarity: Rarity,
    rng: &mut impl Rng,
) -> Result<(Pile, AbInstruction), String> {
    let value_piles: Vec<Pile> = state
        .piles
        .iter()
        .filter(|p| {
            if let PileType::Value {
                price_min_usd,
                price_max_usd,
                rarity,
            } = &p.pile_type
            {
                let effective =
                    rarity.unwrap_or_else(|| price_range_to_rarity(*price_min_usd, *price_max_usd));
                p.estimated_count > 0 && rarity_at_least(effective, target_rarity)
            } else {
                false
            }
        })
        .cloned()
        .collect();

    let use_value = !value_piles.is_empty()
        && (target_rarity == Rarity::Rare
            || target_rarity == Rarity::DoubleRare
            || target_rarity == Rarity::UltraRare)
        && rng.gen_bool(0.7); // 70% use value when we have matching piles and rolled rare+

    let (pile, index) = if use_value {
        let p = value_piles[rng.gen_range(0..value_piles.len())].clone();
        let idx = rng.gen_range(0..p.estimated_count);
        (p, idx)
    } else {
        // Pick from bulk (weighted by count)
        let bulk_piles: Vec<&Pile> = state
            .piles
            .iter()
            .filter(|p| matches!(p.pile_type, PileType::Bulk { .. }) && p.estimated_count > 0)
            .collect();
        if bulk_piles.is_empty() {
            return Err("No bulk piles available".to_string());
        }
        let total: u32 = bulk_piles.iter().map(|p| p.estimated_count).sum();
        if total == 0 {
            return Err("Bulk piles have no cards".to_string());
        }
        let mut roll = rng.gen_range(0..total);
        let mut chosen = bulk_piles[0].clone();
        let mut idx = 0u32;
        for p in bulk_piles {
            if roll < p.estimated_count {
                chosen = p.clone();
                idx = roll;
                break;
            }
            roll -= p.estimated_count;
        }
        (chosen, idx)
    };

    let inst = ab_instruction_for_index(pile.estimated_count, index, rng);
    Ok((pile, inst))
}

fn critical_low_warning(state: &PersistedState) -> Option<String> {
    use crate::models::CRITICAL_LOW_THRESHOLD;
    let low: Vec<&str> = state
        .piles
        .iter()
        .filter(|p| p.estimated_count > 0 && p.estimated_count < CRITICAL_LOW_THRESHOLD)
        .map(|p| p.name.as_str())
        .collect();
    if low.is_empty() {
        None
    } else {
        Some(format!(
            "Some piles are below {} cards; consider refilling or combining: {}",
            CRITICAL_LOW_THRESHOLD,
            low.join(", ")
        ))
    }
}
