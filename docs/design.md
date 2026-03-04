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

- All state in `PersistedState`: `piles`, `settings`, `pack_history`. Saved to a single JSON file (default `./data/state.json`). No auth; single-user localhost.
- **Pack history**: Each generated pack is appended to `pack_history` (newest first). Each entry has `id`, `created_at`, `slots` (with A/B instruction and optional recognized card data). User can open **Pack history** and click a pack to view it at `/pack/result/:id`. From the pack result page or the history list, user can **delete** a pack (POST `/pack/result/:id/delete`); it is removed from history. User can **edit** a pack: for any slot that does not yet have card data, they can enter a Pokemon TCG API card id (e.g. `swsh12-123`) and submit **Look up**; the app fetches card details and stores them in that slot (same as after a scan). Slots that already have card data can be edited (name, holo, Regenerate image) as before.

## Card recognition (optional)

- **Settings**: `image_rec_enabled` (default false), `image_rec_service_url` (optional). When enabled, each slot on the pack result page shows a camera button.
- **Flow**: User taps camera → browser captures image → POST to `/pack/result/:id/slot/:n/scan` → server forwards image to local recognition service (POST `{url}/recognize`), expects JSON `{ "cardId": "setX-nnn" }` → server fetches card details from Pokemon TCG API and stores in that slot. If service URL is unset or request fails, API returns 503 and frontend shows a modal: disable card recognition or keep it on (link to setup guide).
- **Disable**: POST `/settings/image-rec/disable` sets `image_rec_enabled = false`; camera buttons no longer appear.
- **Pack detail**: For each slot, if we have recognized card data we show name, image, holo; user can edit name/holo and **Regenerate image** (refetch from API). For slots without card data, user can add it by scanning (camera) or by entering a card id and **Look up** (fetches from Pokemon TCG API and fills the slot).
- Setup: See **docs/image-recognition-setup.md** and in-app link to `/static/docs/image-recognition-setup.html`.

## Home hero: booster sprite sheet

- `static/images/booster-sheet.webp`: **474 px × 753 px**. Expandable grid in `src/main.rs`: **`HERO_SPRITE_GRID_COLS`** × **`HERO_SPRITE_GRID_ROWS`** (e.g. 2×2, 3×3, 3×5). One random cell (col, row) is shown. **`hero_sprite_edge_types(col, row, cols, rows)`** returns (top, right, bottom, left) as **Inner** (boundary with another cell) or **Outer** (edge of sheet). Corners have 2 outer edges; middle cell in 3×3 has 0; others have 1. **`hero_sprite_cell_rect(col, row, cols, rows)`** uses those to apply trim per edge: **`HERO_SPRITE_TRIM_AT_WIDTH_BOUNDARY`** / **`_HEIGHT`** for inner edges (gap between packs), **`HERO_SPRITE_TRIM_AT_OUTER_WIDTH`** / **`_OUTER_HEIGHT`** for outer edges. start = cell origin + left/top trim; width/height = cell size − left_trim − right_trim (and same for height). Set to **0** for no trim on that edge type. Reload to see a different cell.

## Critical low

- Piles below 40 cards trigger a warning after pack generation (see `CRITICAL_LOW_THRESHOLD` in `models.rs`). User is expected to refill, combine, or enter a clear count.
