# StyleSmart — AI Operating System for Personal Appearance

StyleSmart is a working full-stack web app that helps you get more value out of
what you already own: a wardrobe manager, a daily outfit engine, cost-per-wear
analytics, fit intelligence, a grooming planner, travel packing, a Personal
Appearance Score, smart shopping checks — and a versioned B2B API for retailers.

**Backend: Rust** (axum + tokio + rusqlite). The frontend is a dependency-free
single-page app in `static/` that calls the real JSON API — no build step, no
external CDN assets, works fully offline.

> **Honest limits, up front.** The outfit and fit "AI" in this repo is a
> deterministic rules + heuristics engine (colour harmony tables, weather
> rules, wear-rotation scoring, size matching). It is **not** a vision model,
> **not** an LLM, and it does not look at photos of you or your clothes.
> Everything it recommends is explainable: every pick comes with its reasons.
> The production path — AI vision tagging, retailer integrations, accounts,
> payments, mobile apps — is documented in ARCHITECTURE.md as future
> milestones, not claimed as built.

## Quickstart

```bash
cargo run
# StyleSmart running at http://127.0.0.1:8000
```

Requires a Rust toolchain (rustup: https://rustup.rs). First build downloads
the crates (axum, tokio, serde, rusqlite with bundled SQLite, chrono) and
compiles — a minute or two on first run. Data lives in `wardrobe.db`
(SQLite, created and seeded on first run).

Options:

```bash
cargo run -- --port 8001 --db /path/to/wardrobe.db
cargo run --release   # optimised build
```

## What it does

| Area | What works |
| --- | --- |
| **Dashboard** | Personal Appearance Score (0–100) with a visible breakdown, wardrobe stats, one-tap outfit, recently worn outfits |
| **Wardrobe** | Add / edit / delete items (name, category, colour, brand, price, purchase date, wears, size, fit notes, occasions, material). Filter by category / colour, search, log a wear |
| **What to Wear** | Occasion (work, casual, formal, date, gym, travel) + temperature + condition → a complete outfit with per-item reasoning: colour harmony, weather suitability, rotation (never repeats yesterday's outfit), least-worn preference. "I wore this" logs the outfit and increments wears |
| **Cost-per-Wear** | Per-item cost ÷ wears, total wardrobe value, ROI ranking, most/least worn, a "worth it?" verdict per item |
| **Fit Intelligence** | Your fit profile (sizes per category, slim/regular/relaxed, body notes). Fit score 0–100 per item. Return-risk estimator for a prospective purchase (size match, brand history, category base rate, price, duplicates) |
| **Grooming** | Morning/evening routine products, expiry tracking (expired / expiring-soon warnings), today's checklist with checkboxes that persist |
| **Travel Packing** | Destination, days, weather, occasions → a packing list generated **only** from items you own, plus an essentials checklist |
| **Smart Shopping** | "Do I already own something similar?" — enter category/colour/price → BUY / SKIP / THINK verdict with reasoning |
| **B2B API v1** | `/api/v1/fit-predict` and `/api/v1/recommend`, API-key auth + 30 req/min rate limiting |

Seeded with a realistic demo wardrobe (24 items) and a demo fit profile, so
every feature works on first launch.

## Screenshots (described)

- **Dashboard:** a large Appearance Score number with four breakdown bars
  (completeness, rotation, grooming, cost-per-wear efficiency), wardrobe
  stats, and a one-tap outfit card.
- **Wardrobe:** item cards with a colour swatch, brand, price, wears and
  cost-per-wear, plus Log wear / Edit / Delete actions.
- **What to Wear:** an outfit card listing top, bottom, shoes, outerwear,
  accessory and fragrance, each with bullet-point reasons.
- **Cost-per-Wear:** ROI table ranked cheapest-per-wear first, with verdicts
  from "Excellent value" to "Poor value".
- **Fit Intelligence:** profile form, per-item fit scores, and a return-risk
  estimator showing Low/Medium/High with reasons.
- **Grooming:** today's checklist with checkboxes and expiry badges
  (OK / Expires soon / EXPIRED).
- **Travel Packing:** a checkbox packing list grouped by reason, all from
  your wardrobe.

## Tests

```bash
cargo test
# 59 tests, OK (42 unit + 17 live-API integration tests)
```

The suite covers the outfit engine (all 6 occasions × weather variants,
no-repeat rule, weather suitability, dress-code rules), cost-per-wear math,
fit scoring, return-risk, appearance score, packing logic, shopping checks,
the SQLite layer, and the live HTTP API (spawned server binary on an
ephemeral port with a fresh seeded database per test), including the B2B key
auth and rate limiting.

## API

See [API.md](API.md) for every endpoint with example requests and responses.
B2B demo key: `demo-key-1234` (send as `X-API-Key` header).

## Project layout

```
Cargo.toml                axum 0.7, tokio, serde, rusqlite (bundled), chrono
src/
  main.rs                 entry point: axum router, handlers, static serving,
                          B2B auth + rate limiting, CLI flags
  db.rs                   SQLite schema, seed data, CRUD (no SQL elsewhere)
  engine.rs               outfit engine, CPW, fit, score, packing, shopping
static/
  index.html              single-page app
  app.js                  all features call the real JSON API
  style.css               dark theme, no external assets
tests/
  api.rs                  17 live-API integration tests
README.md  ARCHITECTURE.md  API.md
```

## Roadmap (not built — future milestones)

- M1 AI vision tagging: photo upload → automatic category/colour/material
- M2 Retailer integrations: import purchases from order history
- M3 Accounts, auth, and cloud sync
- M4 Fit prediction trained on real return data
- M5 iOS / Android apps, payments, premium subscription
