# StyleSmart — API Reference

Backend: Rust (axum + tokio + rusqlite). Run it with `cargo run`.

Base URL: `http://127.0.0.1:8000`. All responses are JSON. The frontend calls
these endpoints with `fetch`; nothing is embedded in the page.

## Consumer API (`/api/`)

### Health

```
GET /api/health
→ {"status": "ok", "app": "StyleSmart"}
```

### Wardrobe items

```
GET /api/items
GET /api/items?category=top&color=navy&search=levi
→ [{"id": 1, "name": "White Oxford Shirt", "category": "top", "color": "white",
    "color_hex": "#F5F5F5", "brand": "Charles Tyrwhitt", "price": 79,
    "purchase_date": "2026-01-12", "wears": 14, "last_worn": "2026-11-09",
    "size": "M", "fit_notes": "slim", "occasions": ["work", "formal", "date"],
    "material": "cotton", "created_at": "2026-11-11"}, ...]

GET /api/items/1
→ single item, or 404 {"error": "not found"}

POST /api/items
{"name": "Red Knit Polo", "category": "top", "color": "red", "brand": "Uniqlo",
 "price": 29, "size": "M", "purchase_date": "2026-11-11", "material": "cotton",
 "occasions": ["casual", "date"], "fit_notes": "regular"}
→ 201, the created item. 400 if name or category is missing / category unknown.

PUT /api/items/1
{"wears": 15, "price": 75}
→ the updated item (partial updates; occasions may be a list or CSV string)

DELETE /api/items/1
→ {"deleted": 1}  (404 if missing)

POST /api/items/1/wear
→ the item with wears + 1 and last_worn = today
```

### Outfit engine

```
GET /api/outfit?occasion=work&temp=14&condition=rain
→ {"occasion": "work", "temp": 14.0, "condition": "rain",
    "picks": {"top": {...item}, "bottom": {...}, "footwear": {...},
              "outerwear": {...}, "accessory": {...}, "fragrance": {...}},
    "reasons": {"top": ["Tagged for work.", "Worn 9 times — ...",
                        "Colour harmony 0.90 with the rest of the outfit."]},
    "notes": ["No suitable outerwear in your wardrobe for this weather."],
    "repeated_yesterday": false}

occasion: work | casual | formal | date | gym | travel
condition: sunny | rain | cold | cloudy
400 on an unknown occasion or condition.

POST /api/outfit/log
{"item_ids": [1, 6, 10, 15], "occasion": "work", "temp": 14, "condition": "rain"}
→ {"logged": 4, "date": "2026-11-11"}
Increments wears + updates last_worn for each item and records the outfit
in history (drives the no-repeat rule tomorrow).
```

### Cost-per-wear analytics

```
GET /api/analytics
→ {"total_items": 24, "total_value": 2453.0, "total_wears": 301,
    "average_cost_per_wear": 12.42,
    "items": [{"id": 1, "name": "...", "price": 79, "wears": 14,
               "cost_per_wear": 5.64, "verdict": "Fair — wear it more..."}],
    "roi_ranking": [...same rows, cheapest cost-per-wear first...],
    "most_worn": [{"id": 18, "name": "Silver Watch", "wears": 40}],
    "least_worn": [...],
    "by_category": {"top": {"count": 5, "value": 313, "wears": 63}, ...}}

`average_cost_per_wear` is the mean of `price / max(wears, 1)` over all
items — an item never worn yet counts at its full price. Per-item
`cost_per_wear` is `null` for never-worn items.
```

### Fit intelligence

```
GET /api/fit-profile
→ {"id": 1, "sizes": {"top": "M", "bottom": "32", "footwear": "9",
    "outerwear": "M", "accessory": "M"}, "fit_preference": "slim",
    "body_notes": "Athletic build, ...", "height_cm": 178, "weight_kg": 76.0}

PUT /api/fit-profile
{"sizes": {"top": "L"}, "fit_preference": "regular", "body_notes": "..."}
→ the updated profile (sizes merge with existing)

POST /api/fit-score
{"item_id": 1}
→ {"item_id": 1, "score": 100,
    "breakdown": {"size": 60, "cut": 25, "condition": 15},
    "reasons": ["Size matches your profile exactly.",
                "Cut matches your slim preference.",
                "Worn regularly — proven in practice."]}

POST /api/return-risk
{"category": "footwear", "size": "11", "brand": "NewBrand",
 "price": 200, "color": "red"}
→ {"risk": 85, "band": "High",
    "reasons": ["Footwear has a typical return rate around 15% ...",
                "Size 11 does not match your profile size 9 ...",
                "You own nothing from NewBrand ...",
                "High price raises the stakes ..."]}
```

### Grooming

```
GET /api/grooming
→ {"date": "2026-11-11", "products": [
     {"id": 1, "name": "Vitamin C Serum", "category": "skincare",
      "brand": "The Ordinary", "expiry_date": "2027-03-01",
      "routine": "morning", "notes": "Apply before moisturiser",
      "expiry_status": "ok", "done_today": false}, ...]}
expiry_status: ok | expiring_soon (≤30 days) | expired

POST /api/grooming
{"name": "Face Cleanser", "category": "skincare", "routine": "both",
 "expiry_date": "2027-05-01"}
→ 201, the created product

DELETE /api/grooming/1
→ {"deleted": 1}

POST /api/grooming/check
{"product_id": 1, "done": true, "date": "2026-11-11"}   (date optional, default today)
→ {"date": "2026-11-11", "checks": {"1": true, ...}}

GET /api/grooming/checklist?date=2026-11-11
→ {"date": "2026-11-11", "checks": {"1": true}}
```

### Personal Appearance Score

```
GET /api/score
→ {"score": 47,
    "breakdown": {
      "wardrobe_completeness": {"points": 30, "max": 30,
        "detail": {"top": {"have": 5, "need": 3, "points": 5}, ...}},
      "rotation_variety": {"points": 10, "max": 25, "distinct_recently_worn": 4},
      "grooming_consistency": {"points": 0, "max": 25, "done": 0, "expected": 42},
      "cost_per_wear_efficiency": {"points": 7, "max": 20,
        "efficient_items": 7, "worn_items": 21}}}
```

### Travel packing

```
POST /api/packing
{"destination": "Tokyo", "days": 4, "weather": "rain",
 "occasions": ["casual", "work"]}
→ {"id": 3, "destination": "Tokyo", "days": 4, "weather": "rain",
    "occasions": ["casual", "work"],
    "packed": [{"item_id": 5, "name": "Blue Linen Shirt", "category": "top",
                "qty": 1, "reason": "One top per day (4 days)"},
               {"item_id": null, "name": "Vitamin C Serum", "category": "grooming",
                "qty": 1, "reason": "Grooming (morning routine)"}, ...],
    "total_items": 16,
    "essentials_checklist": ["Passport / ID", "Phone charger", "Travel adapter",
                             "Medications", "Umbrella"],
    "note": "Every packed item comes from your wardrobe — nothing you do not own."}
400 if destination is missing or days is outside 1–30.
Expired grooming products are never packed.

The response also includes an `items` field that is an exact alias of
`packed` — the frontend reads `items`.

GET /api/packings
→ saved packing lists, newest first
```

### Smart shopping

```
POST /api/shopping-check
{"category": "top", "color": "navy", "price": 45}
→ {"verdict": "BUY", "recommended": "buy",
    "similar": [{"id": 2, "name": "Navy Crew T-Shirt", "color": "navy",
                 "wears": 22, "cost_per_wear": 0.68}],
    "reasoning": ["Your Navy Crew T-Shirt is heavily worn (22 wears, $0.68/wear) — a replacement is justified.",
                  "This is a refresh, not a duplicate."]}
Verdicts: BUY (nothing similar, or the similar item is heavily worn),
SKIP (you own a near-duplicate worn < 3 times), THINK (similar item in
regular rotation — wear it more first).
```

### Outfit history

```
GET /api/history
→ [{"date": "2026-11-10", "item_ids": [2, 7, 11, 18], "occasion": "work",
    "temp": 26, "condition": "sunny"}, ...]   (last 14 days)
```

## B2B API v1 (`/api/v1/`)

Requires header `X-API-Key: demo-key-1234`. Without it: `401 {"error":
"unauthorized — send a valid X-API-Key header"}`. Rate limit: 30 requests
per minute per key; beyond that: `429 {"error": "rate limit exceeded — 30
requests per minute"}`.

```
POST /api/v1/fit-predict
{"category": "footwear", "size": "11", "brand": "NewBrand", "price": 200, "color": "red"}
→ {"category": "footwear", "size": "11", "return_risk": 85, "band": "High",
    "reasons": [...]}

POST /api/v1/recommend
{"occasion": "work", "temp": 24, "condition": "sunny"}
→ same outfit object as GET /api/outfit
```
