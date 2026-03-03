# Design

## Pack types

- **Modern**: Implemented. 5-card pack with slot order: 1 Common, 2 Common or Energy (if setting on), 3 Trainer, 4 Uncommon, 5 Rare. Rarity odds per slot are derived from Scarlet & Violet–style data; rare slot has Rare / Double Rare / Ultra Rare weights.
- **Classic / Legacy**: Stubbed in `PackTypeId` and `PackLayout::for_pack_type`; return `None`. Add new layouts by implementing the same layout interface and wiring in the enum.

## Odds and layout

- Layout lives in `src/odds.rs`: `PackLayout`, `SlotOdds`, `SlotRole` (Common, Uncommon, Rare, Energy, Trainer). Each slot has a role and optional rarity weights for rolling.
- Energy slot is optional (Advanced settings, default off). When on, slot 2 is Energy; otherwise slot 2 is Common.
- Price → rarity for value piles: baked-in tiers in `price_rarity_tiers()` map USD ranges to `Rarity`. Value piles can override with explicit `rarity` on the pile.

## Pile types and selection

- **Trainers**: One or more piles of type Trainers; we pick uniformly among them (usually one pile), then A/B within that pile.
- **Energy**: One pile per energy type. We pick type with even likelihood (excluding “out” types), then one card from that pile via A/B.
- **Bulk**: Multiple piles; combined weight by `estimated_count`. We pick a random position in the combined range to choose (pile, index), then A/B for that pile.
- **Value**: Used when a slot rolls Rare+ and we have value piles whose effective rarity (from price or explicit) is at least that. 70% chance to use a matching value pile when available; else we pick from bulk.

## A/B algorithm

- `src/selection.rs`: Given pile size N and target index (or random index), emit a sequence of A (top half) / B (bottom half) until remaining size ≤ 10, then a final number in 2..=10 (or 2..=size). Approximate halving is intentional so the user doesn’t need exact counts.

## State and persistence

- All state in `PersistedState`: `piles`, `settings`. Saved to a single JSON file (default `./data/state.json`). No auth; single-user localhost.

## Critical low

- Piles below 40 cards trigger a warning after pack generation (see `CRITICAL_LOW_THRESHOLD` in `models.rs`). User is expected to refill, combine, or enter a clear count.
