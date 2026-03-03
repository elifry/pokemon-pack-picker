# Pokemon Pack Picker

![Open a Booster Pack](/static/images/open-a-booster-pack.png)

Self-hosted web app to build Pokemon TCG–style packs from your own card piles. Uses official-style slot order and rarity odds, and outputs **A/B halving instructions** so you can quickly locate each card in a physical stack without counting.

- **Pack layout**: Physical slot order matches real packs (e.g. slot 5 = rare). Modern 5-card layout is implemented; Classic/Legacy are stubbed for future use.
- **Piles**: Trainers (one pool, pure random), Energy (per-type even, optional "out" list), Bulk (multiple piles weighted by size), Value (price range → rarity proxy).
- **Running counts**: Estimated pile sizes are decremented after each generated pack. When any pile drops below 40 cards you get a warning to refill or combine.
- **Pile management**: Add, edit, delete, **combine** (merge two piles into one; old piles are deleted), **split** (create a new pile from an existing one with a given count).

## Run locally

Run from the **project root** so static assets (CSS, images) load correctly:

```bash
cargo run
```

Then open **http://127.0.0.1:3000**. The UI uses Pokémon TCG–inspired styling (colors, card-style panels, booster pack imagery). Static files are served from the `static/` directory.

Data is stored in `./data/state.json` by default. Override with env:

```bash
PPP_DATA=/path/to/state.json cargo run
```

## Screenshots

**My Piles** — Manage your card piles: add, edit, delete, combine two piles, or split a pile. Table shows pile name, type, estimated count, and actions.

![My Piles](/static/images/my-piles.png)

**Settings** — Pack format and energy options (cards per pack, pack type, add energy to packs, energy types to exclude). Changes apply to the next pack you open.

![Settings](/static/images/settings.png)

**Your Booster Pack** — After opening a pack, you get slot-by-slot instructions: which pile to use and the A/B halving sequence plus final number. Fill each slot in order so the physical pack matches the intended order. A trainer tip appears if any pile is below 40 cards.

![Your Booster Pack](/static/images/your-booster-pack.png)

## Requirements

- At least one **Trainers** pile, one or more **Bulk** piles, and (if you use value/rarity) **Value** piles with optional price ranges.
- If **Advanced settings** → "Add energy to packs" is on, you need at least one **Energy** pile per type you want; energy types listed in "Energy types to exclude" are skipped.

## How the A/B instructions work

For each slot you get e.g. `A, B, A, A, A, B, A, A — 7`:

- **A** = take the **top half** of the current pile.
- **B** = take the **bottom half** of the current pile.
- When the pile is down to 10 or fewer cards, use the **final number** (2–10) to pick the card (e.g. 7 = 7th from the top of that small stack).

You don't need to split exactly in half; approximate halving is fine. Fill slots in order (Slot 1, then 2, …) so the physical pack matches the intended order.

## License

MIT.
