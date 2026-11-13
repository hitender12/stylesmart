# StyleSmart — Architecture

## Stack

- **Backend:** Rust — axum 0.7 (HTTP routing), tokio (async runtime),
  rusqlite 0.31 with the `bundled` feature (SQLite compiled in, no system
  library needed), serde/serde_json, chrono (local dates).
- **Frontend:** dependency-free single-page app in `static/` — no build step,
  no external CDN assets, works fully offline.
- **Storage:** single-user SQLite file `wardrobe.db`, created and seeded on
  first run.

## Module map

```
src/main.rs
  axum Router (tokio)
  ├─ GET  /  /index.html  /static/*   serves the SPA (no build step)
  ├─ GET  /api/health
  ├─ /api/items, /api/outfit, /api/analytics, /api/fit-profile,
  │  /api/fit-score, /api/return-risk, /api/grooming,
  │  /api/grooming/check, /api/grooming/checklist, /api/score,
  │  /api/packing, /api/packings, /api/shopping-check,
  │  /api/outfit/log, /api/history
  └─ /api/v1/fit-predict, /api/v1/recommend   (X-API-Key + rate limit)
  Shared state: Arc<Mutex<Connection>> (one SQLite connection, locked
  per request — no await is ever held across the lock) and an in-memory
  per-key rate-limit table.
src/db.rs
  SQLite schema + seed data + all CRUD (no SQL outside this module)
src/engine.rs
  Pure functions: no I/O, no DB access — every rule is unit-testable
  ├─ color_harmony, weather_ok, dress_ok
  ├─ build_outfit
  ├─ cost_per_wear, cpw_verdict, analytics
  ├─ fit_score, return_risk
  ├─ appearance_score
  ├─ build_packing
  └─ shopping_check
static/
  index.html / app.js / style.css — the SPA; app.js only talks to /api/*
tests/api.rs — 17 live-API integration tests (spawned binary, fresh DB each)
```

## Data model (SQLite, `wardrobe.db`)

| Table | Key columns |
| --- | --- |
| `items` | id, name, category (top/bottom/footwear/outerwear/accessory/grooming/fragrance), color, color_hex, brand, price, purchase_date, wears, last_worn, size, fit_notes, occasions (CSV), material |
| `fit_profile` | id=1, sizes (JSON per category), fit_preference (slim/regular/relaxed), body_notes, height_cm, weight_kg |
| `grooming` | id, name, category, brand, expiry_date, routine (morning/evening/both), notes |
| `routine_checks` | (date, product_id) → done |
| `outfit_history` | date → item_ids (JSON), occasion, temp, condition |
| `packings` | id, destination, days, weather, occasions, items (JSON), created_at |

Seeded on first run with 24 items, 6 grooming products, one fit profile,
and yesterday's outfit (so the no-repeat rule is demonstrable immediately).

## How the outfit engine scores

`build_outfit(items, occasion, temp, condition, yesterday_ids)` picks one
item per slot (top → bottom → footwear → outerwear → accessory → fragrance)
in that order, so colour harmony can be scored against already-chosen items.

Hard rules (an item is ineligible, not just down-ranked):

- **Weather:** suede and mesh footwear are out in rain; shorts and t-shirts
  are out at ≤12 °C; sweaters/wool and all outerwear are out at ≥28 °C.
- **Dress code:** formal never gets shorts, t-shirts, or trainers; gym
  never gets blazers, ties, or oxfords.
- **Occasion:** an item must be tagged for the occasion to be a first-choice
  candidate. If nothing is tagged, the engine falls back to the closest
  dress-appropriate, weather-appropriate item and says so in `notes`.

Soft scoring per candidate (weights sum to ~1):

- **Wear balance (0.35):** `1 - wears / (max_wears + 1)` — least-worn wins,
  so the wardrobe rotates instead of repeating favourites.
- **Colour harmony (0.45):** average harmony against already-picked items.
  Neutrals pair with anything (1.0), same colour family 0.9, warm+cool 0.4,
  identical top+bottom 0.75. See `color_group` in `engine.rs`.
- **Yesterday penalty (−0.5):** items worn yesterday are excluded when an
  alternative exists; if none exists they can still be picked, flagged in
  the reasons, and `repeated_yesterday` reports an exact-repeat outfit.
- **Temperature nudges (±0.05):** light colours in heat, dark colours in cold.

Outerwear is included when it rains, when temp ≤ 16 °C, or for formal.
Blazer is preferred for mild-weather formal, wool coat for real cold, rain
jacket when raining. Fragrance is skipped for gym; evening/date picks the
woody scent, daytime picks citrus. Accessories: tie for formal/work,
otherwise belt or watch.

Every pick returns its reasons, so the UI can show *why*, not just *what*.

## Fit scoring

`fit_score(item, profile)` — 0–100:

- **Size (60):** exact match 60, one step off 30, far off 0, unknown 30.
  Size steps are ordered lists for letter sizes and numeric distance for
  waist/shoe sizes.
- **Cut (25):** item's `fit_notes` match the profile preference 25,
  no cut recorded 15, mismatch 5.
- **Condition (15):** worn regularly 15, never worn 10, heavily worn (≥30) 5.

`return_risk(...)` starts from a category base rate (footwear 15, bottoms
10, tops/outerwear 8, accessories 5, grooming/fragrance 3), then adds:
size mismatch +45, unknown brand +15, price > $150 +10, same colour
duplicate owned +10. Band: <30 Low, 30–60 Medium, >60 High.

## Appearance Score

- **Wardrobe completeness (30):** 5 points per category target met
  (tops ≥3, bottoms ≥2, footwear ≥2, outerwear ≥1, accessory ≥1,
  fragrance ≥1).
- **Rotation variety (25):** distinct items worn in the last 14 days,
  scaled to a 10-item target.
- **Grooming consistency (25):** routine checks completed in the last 7
  days vs expected (7 days × number of routine products).
- **Cost-per-wear efficiency (20):** share of worn items at ≤ $5/wear.

## B2B API v1

- Auth: `X-API-Key: demo-key-1234` (single demo key; per-key secrets and
  per-plan quotas are a production milestone).
- Rate limiting: 30 requests / 60 s per key, in-memory, 429 beyond that.
- `/api/v1/fit-predict` wraps `return_risk`; `/api/v1/recommend` wraps
  `build_outfit`.

## Port notes (Python → Rust)

The Rust backend is a behaviour-preserving port of the original Python
implementation — same endpoints, same JSON shapes, same engine rules — with
two deliberate differences:

- **Rounding:** Python's `round()` rounds the float's decimal
  representation (half-to-even). Rust's `f64::round()` is half-away-from-zero
  and binary multiplication can land exactly on .5 (e.g. 2.675 × 100).
  `engine::round_half_even` rounds via decimal formatting instead, so
  cost-per-wear and score values match Python exactly (verified by
  `python_rounding_parity` test).
- **Tie-breaking:** Python's `max()` returns the *first* maximum on ties;
  Rust's `Iterator::max_by` returns the *last*. The engine uses a
  first-max helper (`argmax_first`) so picks are identical.
- **Packing response:** `POST /api/packing` returns both `packed` (the
  documented API field) and `items` (an identical alias) — the frontend
  reads `items`, and the Python version returned only `packed`, which broke
  the travel tab's generate flow. The alias fixes that without changing the
  documented shape.
- **Empty wardrobe:** the Python engine crashed on an empty category
  (`max()` of an empty list). The Rust engine skips a slot with no items
  instead of failing the whole outfit.

## Honest limits

- No vision model, no LLM, no photo analysis. Colour/material/occasion
  come from the item record, entered by the user (or the seed).
- No accounts, no auth on the consumer API (localhost use), no payments.
- Single-user SQLite; no cloud sync, no mobile app.
- Fit and return-risk numbers are heuristics from one profile, not a model
  trained on return data.
- The demo B2B key is public by design; rotate it before any real use.

## Future milestones

- M1 AI vision tagging (photo → category/colour/material/occasion tags)
- M2 Retailer integrations (import order history; real return-rate data)
- M3 Accounts, auth, cloud sync, sharing
- M4 Fit model trained on aggregated return data
- M5 Mobile apps, premium subscription, payments
