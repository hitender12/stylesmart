//! StyleSmart — deterministic rules + heuristics engine (Rust port of engine.py).
//!
//! NOT a vision model or LLM. Every recommendation is produced by the
//! scoring rules in this module, so results are reproducible and explainable.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

use crate::db::{GroomingProduct, Item, Profile};

// ---------------------------------------------------------------- colours

pub fn color_group(color: &str) -> &'static str {
    match color.to_lowercase().as_str() {
        "black" | "white" | "grey" | "navy" | "beige" | "brown" => "neutral",
        "blue" | "green" | "teal" | "purple" => "cool",
        "red" | "orange" | "yellow" | "pink" | "camel" | "burgundy" => "warm",
        _ => "neutral",
    }
}

pub fn color_harmony(c1: &str, c2: &str) -> f64 {
    if c1.is_empty() || c2.is_empty() {
        return 0.7;
    }
    let c1 = c1.to_lowercase();
    let c2 = c2.to_lowercase();
    if c1 == c2 {
        return 0.75; // same colour top+bottom is fine but a bit flat
    }
    let g1 = color_group(&c1);
    let g2 = color_group(&c2);
    if g1 == "neutral" || g2 == "neutral" {
        return 1.0;
    }
    if g1 == g2 {
        return 0.9;
    }
    0.4 // warm + cool clash
}

pub const OCCASIONS: [&str; 6] = ["work", "casual", "formal", "date", "gym", "travel"];
pub const CONDITIONS: [&str; 4] = ["sunny", "rain", "cold", "cloudy"];

// Python's round() rounds the float's shortest decimal representation
// (round-half-to-even). Multiplying by 10^digits in binary can land
// exactly on .5 (2.675 * 100 == 267.5 in f64) and give the wrong answer,
// so round via decimal formatting instead — Rust's precision formatting
// is correctly rounded, ties-to-even, on the exact binary value, which
// matches Python's round() for every value in this app's domain.
pub fn round_half_even(x: f64, digits: u32) -> f64 {
    format!("{:.*}", digits as usize, x)
        .parse::<f64>()
        .unwrap_or(x)
}

pub fn round2(x: f64) -> f64 {
    round_half_even(x, 2)
}

pub fn round0(x: f64) -> i64 {
    round_half_even(x, 0) as i64
}

// ---------------------------------------------------------------- outfit engine

pub fn weather_ok(item: &Item, temp: f64, condition: &str) -> (bool, String) {
    let material = item.material.to_lowercase();
    let name = &item.name;
    let category = item.category.as_str();
    if condition == "rain" {
        if material.contains("suede") {
            return (
                false,
                format!("{} is suede — it would be ruined in the rain", name),
            );
        }
        if category == "footwear" && material.contains("mesh") {
            return (
                false,
                format!("{} is mesh — feet would get soaked", name),
            );
        }
    }
    if temp <= 12.0 {
        if category == "bottom" && name.to_lowercase().contains("short") {
            return (false, "shorts are too cold at this temperature".to_string());
        }
        if category == "top" && name.to_lowercase().ends_with("t-shirt") {
            return (
                false,
                "a t-shirt alone is too cold at this temperature".to_string(),
            );
        }
    }
    if temp >= 28.0 {
        if category == "top"
            && (name.to_lowercase().contains("sweater") || material.contains("wool"))
        {
            return (false, format!("{} is too warm for {}°C", name, temp_f(temp)));
        }
        if category == "outerwear" {
            return (false, "outerwear is unnecessary in this heat".to_string());
        }
    }
    (true, String::new())
}

/// Format a temperature the way Python's f-string does for a float
/// (22.0 -> "22", 14.5 -> "14.5").
fn temp_f(temp: f64) -> String {
    if temp == temp.trunc() {
        format!("{}", temp as i64)
    } else {
        format!("{}", temp)
    }
}

fn wear_balance_score(item: &Item, max_wears: i64) -> f64 {
    if max_wears <= 0 {
        return 1.0;
    }
    1.0 - (item.wears as f64 / (max_wears as f64 + 1.0))
}

fn dress_ok(item: &Item, occasion: &str) -> bool {
    let name = item.name.to_lowercase();
    let material = item.material.to_lowercase();
    if occasion == "formal" {
        if name.contains("short") {
            return false;
        }
        if name.contains("t-shirt") {
            return false;
        }
        if item.category == "footwear"
            && (name.contains("sneaker") || name.contains("running") || material.contains("mesh"))
        {
            return false;
        }
    }
    if occasion == "gym" {
        if name.contains("blazer") || name.contains("tie") || name.contains("oxford") {
            return false;
        }
    }
    true
}

fn slot_candidates<'a>(
    items: &'a [Item],
    category: &str,
    occasion: &str,
    temp: f64,
    condition: &str,
) -> Vec<&'a Item> {
    items
        .iter()
        .filter(|it| {
            it.category == category
                && it.occasions.iter().any(|o| o == occasion)
                && dress_ok(it, occasion)
                && weather_ok(it, temp, condition).0
        })
        .collect()
}

/// First-max argmax (mirrors Python's max(), which returns the FIRST
/// maximum on ties — Rust's Iterator::max_by returns the last).
fn argmax_first<'a, F>(pool: &[&'a Item], score: F) -> &'a Item
where
    F: Fn(&Item) -> f64,
{
    let mut best = pool[0];
    let mut best_score = score(best);
    for it in &pool[1..] {
        let s = score(*it);
        if s > best_score {
            best = *it;
            best_score = s;
        }
    }
    best
}

pub fn build_outfit(
    items: &[Item],
    occasion: &str,
    temp: f64,
    condition: &str,
    yesterday_ids: &[i64],
) -> Value {
    let yesterday: HashSet<i64> = yesterday_ids.iter().copied().collect();
    let max_wears = items.iter().map(|i| i.wears).max().unwrap_or(0);
    let mut picked: Vec<(String, Item)> = Vec::new();
    let mut reasons: HashMap<String, Vec<String>> = HashMap::new();
    let mut notes: Vec<String> = Vec::new();

    let needs_outerwear =
        condition == "rain" || temp <= 16.0 || occasion == "formal";
    let wants_fragrance = occasion != "gym";
    let wants_accessory = occasion != "gym";

    let picked_item = |slot: &str, picked: &[(String, Item)]| -> Option<Item> {
        picked.iter().find(|(s, _)| s == slot).map(|(_, it)| it.clone())
    };

    for slot in ["top", "bottom", "footwear"] {
        let mut cands = slot_candidates(items, slot, occasion, temp, condition);
        if cands.is_empty() {
            // Fallback: any item in the category regardless of occasion,
            // but dress-code and weather rules still apply.
            cands = items
                .iter()
                .filter(|i| {
                    i.category == slot && dress_ok(i, occasion) && weather_ok(i, temp, condition).0
                })
                .collect();
            if cands.is_empty() {
                cands = items
                    .iter()
                    .filter(|i| i.category == slot && dress_ok(i, occasion))
                    .collect();
            }
            if cands.is_empty() {
                cands = items.iter().filter(|i| i.category == slot).collect();
            }
            notes.push(format!(
                "No {} is tagged for {} in this weather — used the closest available item.",
                slot, occasion
            ));
        }
        if cands.is_empty() {
            continue; // no item in this category at all; skip the slot
        }
        // Rotation: avoid yesterday's items when an alternative exists.
        let fresh: Vec<&Item> = cands.iter().copied().filter(|c| !yesterday.contains(&c.id)).collect();
        let pool: Vec<&Item> = if fresh.is_empty() { cands } else { fresh };

        let best = {
            let picked_snapshot = picked.clone();
            argmax_first(&pool, |it| {
                let mut s = 0.0;
                s += 0.35 * wear_balance_score(it, max_wears);
                if picked_snapshot.is_empty() {
                    s += 0.45;
                } else {
                    let harm = picked_snapshot
                        .iter()
                        .map(|(_, p)| color_harmony(&it.color, &p.color))
                        .sum::<f64>()
                        / picked_snapshot.len() as f64;
                    s += 0.45 * harm;
                }
                if yesterday.contains(&it.id) {
                    s -= 0.5; // strong penalty, not a hard ban, when no alternative
                }
                // Light colours in heat, darker in cold — small nudge.
                if temp >= 28.0 && ["white", "beige", "blue"].contains(&it.color.as_str()) {
                    s += 0.05;
                }
                if temp <= 12.0 && ["black", "navy", "brown"].contains(&it.color.as_str()) {
                    s += 0.05;
                }
                s
            })
        };
        let mut r = vec![
            format!("Tagged for {}.", occasion),
            format!(
                "Worn {} times — {}.",
                best.wears,
                if best.wears <= 5 {
                    "rotation pick, one of your least-worn"
                } else {
                    "a wardrobe regular"
                }
            ),
        ];
        if !picked.is_empty() {
            let harm = picked
                .iter()
                .map(|(_, p)| color_harmony(&best.color, &p.color))
                .sum::<f64>()
                / picked.len() as f64;
            r.push(format!("Colour harmony {:.2} with the rest of the outfit.", harm));
        }
        if yesterday.contains(&best.id) {
            r.push("You wore this yesterday — it was the only suitable option.".to_string());
        }
        reasons.insert(slot.to_string(), r);
        picked.push((slot.to_string(), best.clone()));
    }

    if needs_outerwear {
        let mut cands = slot_candidates(items, "outerwear", occasion, temp, condition);
        if cands.is_empty() {
            cands = items
                .iter()
                .filter(|i| i.category == "outerwear" && weather_ok(i, temp, condition).0)
                .collect();
        }
        if !cands.is_empty() {
            let fresh: Vec<&Item> = cands.iter().copied().filter(|c| !yesterday.contains(&c.id)).collect();
            let pool: Vec<&Item> = if fresh.is_empty() { cands } else { fresh };
            let top_color = picked_item("top", &picked)
                .map(|t| t.color.clone())
                .unwrap_or_default();
            let best = argmax_first(&pool, |it| {
                let mut s = wear_balance_score(it, max_wears)
                    + 0.3 * color_harmony(&it.color, &top_color);
                let name = it.name.to_lowercase();
                let material = it.material.to_lowercase();
                // Blazer for mild-weather formal, wool coat for real cold,
                // rain jacket when it is actually raining.
                if temp > 16.0
                    && name.contains("blazer")
                    && ["formal", "date", "work"].contains(&occasion)
                {
                    s += 0.25;
                }
                if temp <= 12.0 && (name.contains("coat") || material.contains("wool")) {
                    s += 0.25;
                }
                if condition == "rain" && name.contains("rain") {
                    s += 0.25;
                }
                s
            });
            picked.push(("outerwear".to_string(), best.clone()));
            let why = if condition == "rain" {
                "Rain in the forecast — a jacket is essential."
            } else if temp <= 16.0 {
                "Cold weather — layer up."
            } else {
                "Formal occasions call for a tailored layer."
            };
            reasons.insert(
                "outerwear".to_string(),
                vec![why.to_string(), format!("Worn {} times.", best.wears)],
            );
        } else {
            notes.push("No suitable outerwear in your wardrobe for this weather.".to_string());
        }
    }

    if wants_accessory {
        let cands = slot_candidates(items, "accessory", occasion, temp, condition);
        if !cands.is_empty() {
            let mut pool: Vec<&Item> = if occasion == "formal" || occasion == "work" {
                cands
            } else {
                let no_tie: Vec<&Item> = cands
                    .iter()
                    .copied()
                    .filter(|c| !c.name.to_lowercase().contains("tie"))
                    .collect();
                if no_tie.is_empty() {
                    cands
                } else {
                    no_tie
                }
            };
            let fresh: Vec<&Item> = pool.iter().copied().filter(|c| !yesterday.contains(&c.id)).collect();
            if !fresh.is_empty() {
                pool = fresh;
            }
            let best = argmax_first(&pool, |it| wear_balance_score(it, max_wears));
            picked.push(("accessory".to_string(), best.clone()));
            reasons.insert(
                "accessory".to_string(),
                vec![
                    format!("Finishing touch for {}.", occasion),
                    format!("Worn {} times.", best.wears),
                ],
            );
        }
    }

    if wants_fragrance {
        let cands = slot_candidates(items, "fragrance", occasion, temp, condition);
        if !cands.is_empty() {
            let evening = occasion == "date" || occasion == "formal";
            let mut pool: Vec<&Item> = cands
                .iter()
                .copied()
                .filter(|c| {
                    (c.name.to_lowercase().contains("woody")
                        || c.name.to_lowercase().contains("evening"))
                        == evening
                })
                .collect();
            if pool.is_empty() {
                pool = cands;
            }
            let fresh: Vec<&Item> = pool.iter().copied().filter(|c| !yesterday.contains(&c.id)).collect();
            if !fresh.is_empty() {
                pool = fresh;
            }
            let best = argmax_first(&pool, |it| wear_balance_score(it, max_wears));
            picked.push(("fragrance".to_string(), best.clone()));
            reasons.insert(
                "fragrance".to_string(),
                vec![
                    if evening {
                        "Evening scent for the occasion.".to_string()
                    } else {
                        "Light daytime scent.".to_string()
                    },
                    format!("Worn {} times.", best.wears),
                ],
            );
        }
    }

    // Did we repeat yesterday's exact outfit?
    let picked_ids: HashSet<i64> = picked.iter().map(|(_, it)| it.id).collect();
    let repeated = !yesterday.is_empty() && picked_ids == yesterday;

    let mut picks = serde_json::Map::new();
    for (slot, it) in &picked {
        picks.insert(slot.clone(), serde_json::to_value(it).unwrap());
    }
    let mut reasons_map = serde_json::Map::new();
    for (slot, r) in &reasons {
        reasons_map.insert(slot.clone(), json!(r));
    }
    json!({
        "occasion": occasion,
        "temp": temp,
        "condition": condition,
        "picks": picks,
        "reasons": reasons_map,
        "notes": notes,
        "repeated_yesterday": repeated,
    })
}

// ---------------------------------------------------------------- cost-per-wear

pub fn cost_per_wear(item: &Item) -> Option<f64> {
    if item.wears <= 0 {
        return None; // never worn: no CPW yet
    }
    Some(item.price / item.wears as f64)
}

pub fn cpw_verdict(cpw: Option<f64>) -> &'static str {
    match cpw {
        None => "Not worn yet — wear it to start earning value",
        Some(v) if v < 2.0 => "Excellent value — pennies per wear",
        Some(v) if v < 5.0 => "Good value",
        Some(v) if v < 10.0 => "Fair — wear it more to bring the cost down",
        Some(_) => "Poor value — wear it more, or reconsider keeping it",
    }
}

pub fn analytics(items: &[Item]) -> Value {
    let total_value = round2(items.iter().map(|i| i.price).sum::<f64>());
    let total_wears: i64 = items.iter().map(|i| i.wears).sum();
    let mut rows: Vec<Value> = Vec::new();
    for it in items {
        let cpw = cost_per_wear(it);
        rows.push(json!({
            "id": it.id,
            "name": it.name,
            "category": it.category,
            "price": it.price,
            "wears": it.wears,
            "cost_per_wear": cpw.map(round2),
            "verdict": cpw_verdict(cpw),
        }));
    }
    let mut roi: Vec<Value> = rows
        .iter()
        .filter(|r| !r["cost_per_wear"].is_null())
        .cloned()
        .collect();
    roi.sort_by(|a, b| {
        a["cost_per_wear"]
            .as_f64()
            .unwrap()
            .partial_cmp(&b["cost_per_wear"].as_f64().unwrap())
            .unwrap()
    });
    let mut most_worn: Vec<&Item> = items.iter().collect();
    most_worn.sort_by(|a, b| b.wears.cmp(&a.wears));
    let most_worn: Vec<Value> = most_worn
        .iter()
        .take(5)
        .map(|i| json!({"id": i.id, "name": i.name, "wears": i.wears}))
        .collect();
    let mut least_worn: Vec<&Item> = items.iter().filter(|i| i.wears > 0).collect();
    least_worn.sort_by(|a, b| a.wears.cmp(&b.wears));
    let least_worn: Vec<Value> = least_worn
        .iter()
        .take(5)
        .map(|i| json!({"id": i.id, "name": i.name, "wears": i.wears}))
        .collect();
    let mut by_category = serde_json::Map::new();
    for it in items {
        let entry = by_category
            .entry(it.category.clone())
            .or_insert_with(|| json!({"count": 0, "value": 0.0, "wears": 0}));
        let count = entry["count"].as_i64().unwrap() + 1;
        let value = entry["value"].as_f64().unwrap() + it.price;
        let wears = entry["wears"].as_i64().unwrap() + it.wears;
        entry["count"] = json!(count);
        entry["value"] = json!(value);
        entry["wears"] = json!(wears);
    }
    // Average cost-per-wear across ALL items: an item never worn yet
    // costs its full price per wear so far (price / max(wears, 1)).
    // This matches the documented API example (12.42 on the seed data);
    // averaging only worn items would understate it (11.62).
    let average_cost_per_wear = if items.is_empty() {
        Value::Null
    } else {
        let sum: f64 = items
            .iter()
            .map(|i| i.price / (i.wears.max(1) as f64))
            .sum();
        json!(round2(sum / items.len() as f64))
    };
    json!({
        "total_items": items.len(),
        "total_value": total_value,
        "total_wears": total_wears,
        "average_cost_per_wear": average_cost_per_wear,
        "items": rows,
        "roi_ranking": roi,
        "most_worn": most_worn,
        "least_worn": least_worn,
        "by_category": by_category,
    })
}

// ---------------------------------------------------------------- fit intelligence

fn size_order(category: &str) -> Option<&'static [&'static str]> {
    match category {
        "top" | "outerwear" | "accessory" => {
            Some(&["XS", "S", "M", "L", "XL", "XXL"])
        }
        _ => None, // numeric waist / shoe sizes
    }
}

fn size_distance(category: &str, item_size: &str, profile_size: &str) -> i64 {
    if item_size.is_empty() || profile_size.is_empty() {
        return 99;
    }
    if let Some(order) = size_order(category) {
        let a = order.iter().position(|s| s.eq_ignore_ascii_case(item_size));
        let b = order.iter().position(|s| s.eq_ignore_ascii_case(profile_size));
        return match (a, b) {
            (Some(a), Some(b)) => (a as i64 - b as i64).abs(),
            _ => 99,
        };
    }
    match (item_size.parse::<f64>(), profile_size.parse::<f64>()) {
        (Ok(a), Ok(b)) => (a - b).abs() as i64,
        _ => 99,
    }
}

pub fn fit_score(item: &Item, profile: &Profile) -> Value {
    let pref = profile.fit_preference.to_lowercase();
    let category = item.category.as_str();
    let profile_size = profile
        .sizes
        .get(category)
        .cloned()
        .unwrap_or_default();
    let dist = size_distance(category, &item.size, &profile_size);
    let (size_pts, size_note) = if dist == 0 {
        (60, "Size matches your profile exactly.".to_string())
    } else if dist == 1 {
        (30, "One size off your profile — may still work.".to_string())
    } else if dist == 99 {
        (30, "No size on record to compare — assumed neutral.".to_string())
    } else {
        (
            0,
            "Size is far from your profile — likely a poor fit.".to_string(),
        )
    };
    let notes = item.fit_notes.to_lowercase();
    let (fit_pts, fit_note) = if !pref.is_empty() && notes.contains(&pref) {
        (25, format!("Cut matches your {} preference.", pref))
    } else if notes.is_empty() {
        (15, "No cut recorded — assumed neutral.".to_string())
    } else {
        (
            5,
            format!("Cut is {}, you prefer {}.", notes, pref),
        )
    };
    let wears = item.wears;
    let (cond_pts, cond_note) = if wears >= 30 {
        (5, "Heavily worn — may be past its best.".to_string())
    } else if wears == 0 {
        (10, "Never worn — fit unproven in practice.".to_string())
    } else {
        (15, "Worn regularly — proven in practice.".to_string())
    };
    let total = (size_pts + fit_pts + cond_pts).clamp(0, 100);
    json!({
        "item_id": item.id,
        "score": total,
        "breakdown": {"size": size_pts, "cut": fit_pts, "condition": cond_pts},
        "reasons": [size_note, fit_note, cond_note],
    })
}

fn title_case(s: &str) -> String {
    let mut out = String::new();
    let mut cap = true;
    for ch in s.chars() {
        if ch.is_whitespace() {
            cap = true;
            out.push(ch);
        } else if cap {
            out.extend(ch.to_uppercase());
            cap = false;
        } else {
            out.push(ch);
        }
    }
    out
}

pub fn return_risk(
    category: &str,
    size: &str,
    brand: &str,
    price: f64,
    color: &str,
    items: &[Item],
    profile: &Profile,
) -> Value {
    let mut reasons: Vec<String> = Vec::new();
    let mut risk: i64 = 0;
    let base = match category {
        "footwear" => 15,
        "bottom" => 10,
        "top" | "outerwear" => 8,
        "accessory" => 5,
        "grooming" | "fragrance" => 3,
        _ => 8,
    };
    risk += base;
    reasons.push(format!(
        "{} has a typical return rate around {}% for fit reasons.",
        title_case(category),
        base
    ));
    let profile_size = profile
        .sizes
        .get(category)
        .cloned()
        .unwrap_or_default();
    let dist = size_distance(category, size, &profile_size);
    if dist == 0 {
        reasons.push("Size matches your profile — low risk from sizing.".to_string());
    } else if dist == 99 {
        risk += 15;
        reasons.push("No size on record — sizing is a guess.".to_string());
    } else {
        risk += 45;
        reasons.push(format!(
            "Size {} does not match your profile size {} — the top reason for returns.",
            size, profile_size
        ));
    }
    let owned_brands: HashSet<String> = items.iter().map(|i| i.brand.to_lowercase()).collect();
    if !brand.is_empty() && !owned_brands.contains(&brand.to_lowercase()) {
        risk += 15;
        reasons.push(format!(
            "You own nothing from {} — its sizing is unproven for you.",
            brand
        ));
    }
    if price > 150.0 {
        risk += 10;
        reasons.push("High price raises the stakes — a wrong fit costs more.".to_string());
    }
    let dupes: Vec<&Item> = items
        .iter()
        .filter(|i| i.category == category && i.color.to_lowercase() == color.to_lowercase())
        .collect();
    if let Some(first) = dupes.first() {
        risk += 10;
        reasons.push(format!(
            "You already own {} in the same colour — a duplicate is easy to regret.",
            first.name
        ));
    }
    let risk = risk.clamp(0, 100);
    let band = if risk < 30 {
        "Low"
    } else if risk <= 60 {
        "Medium"
    } else {
        "High"
    };
    json!({"risk": risk, "band": band, "reasons": reasons})
}

// ---------------------------------------------------------------- appearance score

pub fn appearance_score(
    items: &[Item],
    recent_outfit_item_ids: &HashSet<i64>,
    grooming_done: i64,
    grooming_expected: i64,
) -> Value {
    // Completeness (30): one point per category-depth target met.
    let targets: [(&str, i64); 6] = [
        ("top", 3),
        ("bottom", 2),
        ("footwear", 2),
        ("outerwear", 1),
        ("accessory", 1),
        ("fragrance", 1),
    ];
    let mut counts: HashMap<&str, i64> = HashMap::new();
    for it in items {
        *counts.entry(it.category.as_str()).or_insert(0) += 1;
    }
    let mut completeness: i64 = 0;
    let mut comp_detail = serde_json::Map::new();
    for (cat, need) in targets {
        let have = counts.get(cat).copied().unwrap_or(0);
        let pts = if have >= need {
            5
        } else {
            round0(5.0 * have as f64 / need as f64)
        };
        completeness += pts;
        comp_detail.insert(
            cat.to_string(),
            json!({"have": have, "need": need, "points": pts}),
        );
    }
    // Rotation variety (25): distinct items worn recently vs a 10-item target.
    let variety = (recent_outfit_item_ids.len() as f64 / 10.0).min(1.0);
    let rotation = round0(25.0 * variety);
    // Grooming consistency (25): completed routine slots vs expected.
    let grooming = if grooming_expected > 0 {
        round0(25.0 * ((grooming_done as f64 / grooming_expected as f64).min(1.0)))
    } else {
        0
    };
    // Cost-per-wear efficiency (20): share of worn items at <= $5/wear.
    let worn: Vec<&Item> = items.iter().filter(|i| i.wears > 0).collect();
    let efficient: Vec<&&Item> = worn
        .iter()
        .filter(|i| cost_per_wear(i).unwrap_or(999.0) <= 5.0)
        .collect();
    let efficiency = if worn.is_empty() {
        0
    } else {
        round0(20.0 * (efficient.len() as f64 / worn.len() as f64))
    };
    let total = completeness + rotation + grooming + efficiency;
    json!({
        "score": total,
        "breakdown": {
            "wardrobe_completeness": {"points": completeness, "max": 30, "detail": comp_detail},
            "rotation_variety": {"points": rotation, "max": 25,
                                 "distinct_recently_worn": recent_outfit_item_ids.len()},
            "grooming_consistency": {"points": grooming, "max": 25,
                                     "done": grooming_done, "expected": grooming_expected},
            "cost_per_wear_efficiency": {"points": efficiency, "max": 20,
                                         "efficient_items": efficient.len(),
                                         "worn_items": worn.len()},
        },
    })
}

// ---------------------------------------------------------------- travel packing

fn pick_into(
    packed: &mut Vec<Value>,
    items: &[Item],
    category: &str,
    n: i64,
    reason: &str,
    occasion_filter: Option<&[String]>,
    temp: f64,
    condition: &str,
) {
    let mut cands: Vec<&Item> = items.iter().filter(|i| i.category == category).collect();
    if let Some(filter) = occasion_filter {
        let filtered: Vec<&Item> = cands
            .iter()
            .copied()
            .filter(|i| i.occasions.iter().any(|o| filter.contains(o)))
            .collect();
        if !filtered.is_empty() {
            cands = filtered;
        }
    }
    let mut cands: Vec<&Item> = cands
        .into_iter()
        .filter(|i| weather_ok(i, temp, condition).0)
        .collect();
    cands.sort_by(|a, b| (a.wears, a.name.clone()).cmp(&(b.wears, b.name.clone())));
    for it in cands.into_iter().take(n.max(0) as usize) {
        packed.push(json!({
            "item_id": it.id,
            "name": it.name,
            "category": category,
            "qty": 1,
            "reason": reason,
        }));
    }
}

pub fn build_packing(
    items: &[Item],
    grooming_products: &[GroomingProduct],
    destination: &str,
    days: i64,
    weather: &str,
    occasions: &[String],
    today: &str,
) -> Value {
    let mut occ: Vec<String> = occasions
        .iter()
        .filter(|o| OCCASIONS.contains(&o.as_str()))
        .cloned()
        .collect();
    if occ.is_empty() {
        occ = vec!["casual".to_string()];
    }
    let mut temp: f64 = 20.0;
    let mut condition = "sunny";
    let w = weather.to_lowercase();
    if w.contains("rain") {
        condition = "rain";
    } else if w.contains("cold") || w.contains("snow") {
        condition = "cold";
        temp = 8.0;
    } else if w.contains("hot") {
        temp = 30.0;
    }
    let mut packed: Vec<Value> = Vec::new();

    pick_into(&mut packed, items, "top", days.min(7),
              &format!("One top per day ({} days)", days), Some(&occ), temp, condition);
    pick_into(&mut packed, items, "bottom", (days + 1) / 2,
              "One bottom per two days", Some(&occ), temp, condition);
    let shoe_occasions: HashSet<&str> = occ.iter().map(|s| s.as_str()).collect();
    let mut shoe_slots: Vec<&str> = Vec::new();
    if shoe_occasions.contains("formal") || shoe_occasions.contains("date") {
        shoe_slots.push("formal");
    }
    if shoe_occasions.contains("gym") {
        shoe_slots.push("gym");
    }
    shoe_slots.push("casual");
    let mut seen_shoes: HashSet<i64> = HashSet::new();
    for occ_name in shoe_slots {
        let mut cands: Vec<&Item> = items
            .iter()
            .filter(|i| {
                i.category == "footwear"
                    && i.occasions.iter().any(|o| o == occ_name)
                    && weather_ok(i, temp, condition).0
                    && !seen_shoes.contains(&i.id)
            })
            .collect();
        if !cands.is_empty() {
            cands.sort_by(|a, b| a.wears.cmp(&b.wears));
            let it = cands[0];
            seen_shoes.insert(it.id);
            packed.push(json!({
                "item_id": it.id,
                "name": it.name,
                "category": "footwear",
                "qty": 1,
                "reason": format!("Shoes for {} occasions", occ_name),
            }));
        }
    }
    if condition == "rain" || condition == "cold" || temp <= 16.0 {
        pick_into(
            &mut packed,
            items,
            "outerwear",
            1,
            "Weather protection for the trip",
            Some(&occ),
            temp,
            condition,
        );
    }
    pick_into(
        &mut packed,
        items,
        "accessory",
        1,
        "One versatile accessory",
        Some(&occ),
        temp,
        condition,
    );
    if occ.iter().any(|o| o == "date" || o == "formal") {
        pick_into(
            &mut packed,
            items,
            "fragrance",
            1,
            "Evening fragrance",
            Some(&["date".to_string(), "formal".to_string()]),
            temp,
            condition,
        );
    } else {
        pick_into(
            &mut packed,
            items,
            "fragrance",
            1,
            "Everyday fragrance",
            Some(&occ),
            temp,
            condition,
        );
    }
    // Grooming: non-expired routine products only.
    for g in grooming_products {
        if !g.expiry_date.is_empty() && g.expiry_date.as_str() < today {
            continue;
        }
        packed.push(json!({
            "item_id": Value::Null,
            "name": g.name,
            "category": "grooming",
            "qty": 1,
            "reason": format!("Grooming ({} routine)", g.routine),
        }));
    }
    let essentials: Vec<&str> = if condition == "sunny" {
        vec![
            "Passport / ID",
            "Phone charger",
            "Travel adapter",
            "Medications",
            "Sunglasses",
        ]
    } else {
        vec![
            "Passport / ID",
            "Phone charger",
            "Travel adapter",
            "Medications",
            "Umbrella",
        ]
    };
    let total_items: i64 = packed.iter().map(|p| p["qty"].as_i64().unwrap_or(0)).sum();
    json!({
        "destination": destination,
        "days": days,
        "weather": weather,
        "occasions": occ,
        "packed": packed,
        "total_items": total_items,
        "essentials_checklist": essentials,
        "note": "Every packed item comes from your wardrobe — nothing you do not own.",
    })
}

// ---------------------------------------------------------------- smart shopping

fn cpw_or_none(it: &Item) -> Value {
    match cost_per_wear(it) {
        Some(c) => json!(round2(c)),
        None => Value::Null,
    }
}

fn cpw_text(it: &Item) -> String {
    match cost_per_wear(it) {
        Some(c) => format!("${:.2}/wear", c),
        None => "never worn".to_string(),
    }
}

pub fn shopping_check(items: &[Item], category: &str, color: &str, price: f64) -> Value {
    let _ = price; // price is accepted for API parity; the verdict uses wear counts
    let mut exact: Vec<&Item> = Vec::new();
    let mut family: Vec<&Item> = Vec::new();
    for it in items {
        if it.category != category {
            continue;
        }
        let same_color = it.color.to_lowercase() == color.to_lowercase();
        let same_family = color_group(&it.color) == color_group(color);
        if same_color {
            exact.push(it);
        } else if same_family {
            family.push(it);
        }
    }
    let mut similar: Vec<&Item> = Vec::new();
    {
        let mut e = exact.clone();
        e.sort_by(|a, b| a.wears.cmp(&b.wears));
        similar.extend(e);
        let mut f = family.clone();
        f.sort_by(|a, b| a.wears.cmp(&b.wears));
        similar.extend(f);
    }
    let similar_json: Vec<Value> = similar
        .iter()
        .take(3)
        .map(|it| {
            json!({
                "id": it.id,
                "name": it.name,
                "color": it.color,
                "wears": it.wears,
                "cost_per_wear": cpw_or_none(it),
            })
        })
        .collect();
    if exact.is_empty() {
        let mut reasoning = vec![format!(
            "No {} in {} in your wardrobe — this would add something new.",
            category, color
        )];
        if let Some(first) = family.first() {
            reasoning.push(format!(
                "Closest you own: {} ({}) — same colour family, but a different colour.",
                first.name, first.color
            ));
        }
        return json!({
            "verdict": "BUY",
            "similar": similar_json,
            "reasoning": reasoning,
            "recommended": "buy",
        });
    }
    let min_wears = exact.iter().map(|i| i.wears).min().unwrap_or(0);
    let max_wears = exact.iter().map(|i| i.wears).max().unwrap_or(0);
    let least = exact.iter().min_by_key(|i| i.wears).unwrap();
    let most = exact.iter().max_by_key(|i| i.wears).unwrap();
    let (verdict, rec, why): (&str, &str, Vec<String>) = if min_wears < 3 {
        (
            "SKIP",
            "skip",
            vec![
                format!(
                    "You already own {} ({}, {}) — worn only {} times. Wear what you own first.",
                    least.name,
                    least.color,
                    cpw_text(least),
                    least.wears
                ),
                "A near-duplicate would add clutter, not value.".to_string(),
            ],
        )
    } else if max_wears >= 10 {
        (
            "BUY",
            "buy",
            vec![
                format!(
                    "Your {} is heavily worn ({} wears, {}) — a replacement is justified.",
                    most.name,
                    most.wears,
                    cpw_text(most)
                ),
                "This is a refresh, not a duplicate.".to_string(),
            ],
        )
    } else {
        (
            "THINK",
            "think",
            vec![
                format!(
                    "You own {} ({}, {}) — similar and still in good rotation ({} wears).",
                    least.name,
                    least.color,
                    cpw_text(least),
                    least.wears
                ),
                "Try wearing it for two weeks before buying another.".to_string(),
            ],
        )
    };
    json!({
        "verdict": verdict,
        "recommended": rec,
        "similar": similar_json,
        "reasoning": why,
    })
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Item;

    pub fn demo_wardrobe() -> Vec<Item> {
        let it = Item::default();
        vec![
            Item { id: 1, name: "White Shirt".into(), category: "top".into(), color: "white".into(), occasions: vec!["work".into(), "formal".into(), "date".into()], size: "M".into(), wears: 5, fit_notes: "slim".into(), price: 80.0, ..it.clone() },
            Item { id: 2, name: "Navy Tee".into(), category: "top".into(), color: "navy".into(), occasions: vec!["casual".into(), "gym".into(), "travel".into()], size: "M".into(), wears: 2, fit_notes: "regular".into(), price: 15.0, ..it.clone() },
            Item { id: 3, name: "Merino Sweater".into(), category: "top".into(), color: "grey".into(), occasions: vec!["work".into(), "casual".into()], size: "M".into(), wears: 1, fit_notes: "regular".into(), price: 90.0, material: "merino wool".into(), ..it.clone() },
            Item { id: 4, name: "Chinos".into(), category: "bottom".into(), color: "navy".into(), occasions: vec!["work".into(), "casual".into(), "date".into()], size: "32".into(), wears: 4, fit_notes: "slim".into(), price: 60.0, ..it.clone() },
            Item { id: 5, name: "Jeans".into(), category: "bottom".into(), color: "blue".into(), occasions: vec!["casual".into(), "date".into(), "travel".into()], size: "32".into(), wears: 10, fit_notes: "regular".into(), price: 90.0, ..it.clone() },
            Item { id: 6, name: "Shorts".into(), category: "bottom".into(), color: "grey".into(), occasions: vec!["casual".into(), "gym".into(), "travel".into()], size: "M".into(), wears: 3, fit_notes: "relaxed".into(), price: 45.0, ..it.clone() },
            Item { id: 7, name: "Oxfords".into(), category: "footwear".into(), color: "brown".into(), occasions: vec!["work".into(), "formal".into(), "date".into()], size: "9".into(), wears: 6, fit_notes: "regular".into(), price: 130.0, material: "leather".into(), ..it.clone() },
            Item { id: 8, name: "Sneakers".into(), category: "footwear".into(), color: "white".into(), occasions: vec!["casual".into(), "travel".into(), "gym".into()], size: "9".into(), wears: 8, fit_notes: "regular".into(), price: 110.0, material: "leather".into(), ..it.clone() },
            Item { id: 9, name: "Suede Loafers".into(), category: "footwear".into(), color: "brown".into(), occasions: vec!["casual".into(), "date".into()], size: "9".into(), wears: 1, fit_notes: "regular".into(), price: 160.0, material: "suede".into(), ..it.clone() },
            Item { id: 10, name: "Blazer".into(), category: "outerwear".into(), color: "navy".into(), occasions: vec!["work".into(), "formal".into(), "date".into()], size: "M".into(), wears: 2, fit_notes: "slim".into(), price: 200.0, material: "wool".into(), ..it.clone() },
            Item { id: 11, name: "Rain Jacket".into(), category: "outerwear".into(), color: "black".into(), occasions: vec!["casual".into(), "travel".into(), "work".into()], size: "M".into(), wears: 1, fit_notes: "regular".into(), price: 70.0, material: "nylon".into(), ..it.clone() },
            Item { id: 12, name: "Belt".into(), category: "accessory".into(), color: "brown".into(), occasions: vec!["work".into(), "formal".into(), "casual".into(), "date".into()], size: "M".into(), wears: 5, price: 80.0, ..it.clone() },
            Item { id: 13, name: "Watch".into(), category: "accessory".into(), color: "grey".into(), occasions: vec!["work".into(), "formal".into(), "casual".into(), "date".into(), "travel".into()], size: "M".into(), wears: 12, price: 300.0, ..it.clone() },
            Item { id: 14, name: "Citrus EDT".into(), category: "fragrance".into(), color: "yellow".into(), occasions: vec!["casual".into(), "date".into(), "work".into(), "travel".into()], size: "".into(), wears: 4, price: 100.0, ..it.clone() },
            Item { id: 15, name: "Woody EDP".into(), category: "fragrance".into(), color: "black".into(), occasions: vec!["date".into(), "formal".into(), "work".into()], size: "".into(), wears: 2, price: 165.0, ..{ it }.clone() },
        ]
    }

    pub fn profile() -> Profile {
        Profile::default()
    }

    #[test]
    fn neutral_matches_everything() {
        assert_eq!(color_harmony("navy", "white"), 1.0);
        assert_eq!(color_harmony("black", "red"), 1.0);
    }

    #[test]
    fn same_family_high() {
        assert!(color_harmony("blue", "teal") > 0.8);
    }

    #[test]
    fn warm_cool_clash_low() {
        assert!(color_harmony("red", "blue") < 0.5);
    }

    #[test]
    fn suede_blocked_in_rain() {
        let it = Item { material: "suede".into(), ..Item::default() };
        assert!(!weather_ok(&it, 20.0, "rain").0);
    }

    #[test]
    fn mesh_blocked_in_rain() {
        let it = Item { category: "footwear".into(), material: "mesh".into(), ..Item::default() };
        assert!(!weather_ok(&it, 20.0, "rain").0);
    }

    #[test]
    fn shorts_blocked_in_cold() {
        let it = Item { name: "Grey Shorts".into(), category: "bottom".into(), ..Item::default() };
        assert!(!weather_ok(&it, 5.0, "cold").0);
    }

    #[test]
    fn sweater_blocked_in_heat() {
        let it = Item { name: "Merino Sweater".into(), material: "merino wool".into(), ..Item::default() };
        assert!(!weather_ok(&it, 32.0, "sunny").0);
    }

    #[test]
    fn outerwear_blocked_in_heat() {
        let it = Item { category: "outerwear".into(), ..Item::default() };
        assert!(!weather_ok(&it, 30.0, "sunny").0);
    }

    #[test]
    fn normal_items_fine() {
        let it = Item { material: "cotton".into(), ..Item::default() };
        assert!(weather_ok(&it, 22.0, "sunny").0);
    }

    #[test]
    fn all_occasions_produce_complete_outfit() {
        let w = demo_wardrobe();
        for occ in OCCASIONS {
            let o = build_outfit(&w, occ, 22.0, "sunny", &[]);
            assert!(o["picks"]["top"].is_object(), "{} missing top", occ);
            assert!(o["picks"]["bottom"].is_object(), "{} missing bottom", occ);
            assert!(o["picks"]["footwear"].is_object(), "{} missing footwear", occ);
            for (slot, it) in o["picks"].as_object().unwrap() {
                let tagged = it["occasions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v.as_str() == Some(occ));
                if !tagged {
                    let notes = o["notes"].as_array().unwrap();
                    assert!(
                        notes.iter().any(|n| n.as_str().unwrap().contains("closest available")),
                        "{} {} not tagged for {} and no fallback note",
                        slot,
                        it["name"],
                        occ
                    );
                }
            }
            if occ == "formal" {
                assert!(!o["picks"]["bottom"]["name"].as_str().unwrap().contains("Shorts"));
                assert!(!o["picks"]["top"]["name"].as_str().unwrap().contains("T-Shirt"));
            }
        }
    }

    #[test]
    fn gym_has_no_fragrance_or_accessory() {
        let o = build_outfit(&demo_wardrobe(), "gym", 22.0, "sunny", &[]);
        assert!(o["picks"]["fragrance"].is_null());
        assert!(o["picks"]["accessory"].is_null());
    }

    #[test]
    fn formal_gets_outerwear_and_evening_fragrance() {
        let o = build_outfit(&demo_wardrobe(), "formal", 20.0, "sunny", &[]);
        assert!(o["picks"]["outerwear"].is_object());
        assert!(o["picks"]["fragrance"].is_object());
        assert!(o["picks"]["fragrance"]["name"]
            .as_str()
            .unwrap()
            .contains("Woody"));
    }

    #[test]
    fn rain_excludes_suede_and_adds_jacket() {
        let o = build_outfit(&demo_wardrobe(), "casual", 18.0, "rain", &[]);
        assert_ne!(o["picks"]["footwear"]["material"].as_str().unwrap(), "suede");
        assert!(o["picks"]["outerwear"].is_object());
        assert_eq!(
            o["picks"]["outerwear"]["name"].as_str().unwrap(),
            "Rain Jacket"
        );
    }

    #[test]
    fn cold_excludes_shorts_and_adds_outerwear() {
        let o = build_outfit(&demo_wardrobe(), "casual", 5.0, "cold", &[]);
        assert!(!o["picks"]["bottom"]["name"].as_str().unwrap().contains("Shorts"));
        assert!(o["picks"]["outerwear"].is_object());
    }

    #[test]
    fn heat_excludes_outerwear_and_sweater() {
        let o = build_outfit(&demo_wardrobe(), "casual", 32.0, "sunny", &[]);
        assert!(o["picks"]["outerwear"].is_null());
        assert!(!o["picks"]["top"]["name"].as_str().unwrap().contains("Sweater"));
    }

    #[test]
    fn no_repeat_yesterday() {
        let yesterday = [2, 5, 8];
        let o = build_outfit(&demo_wardrobe(), "casual", 22.0, "sunny", &yesterday);
        let picked: HashSet<i64> = o["picks"]
            .as_object()
            .unwrap()
            .values()
            .map(|v| v["id"].as_i64().unwrap())
            .collect();
        assert_ne!(picked, yesterday.iter().copied().collect::<HashSet<i64>>());
        assert!(picked.is_disjoint(&yesterday.iter().copied().collect()));
    }

    #[test]
    fn least_worn_preferred() {
        let w = vec![
            Item { id: 1, name: "Shirt A".into(), category: "top".into(), color: "white".into(), occasions: vec!["casual".into()], wears: 20, ..Item::default() },
            Item { id: 2, name: "Shirt B".into(), category: "top".into(), color: "white".into(), occasions: vec!["casual".into()], wears: 0, ..Item::default() },
            Item { id: 3, name: "Jeans".into(), category: "bottom".into(), color: "blue".into(), occasions: vec!["casual".into()], wears: 0, ..Item::default() },
            Item { id: 4, name: "Sneakers".into(), category: "footwear".into(), color: "white".into(), occasions: vec!["casual".into()], wears: 0, ..Item::default() },
        ];
        let o = build_outfit(&w, "casual", 22.0, "sunny", &[]);
        assert_eq!(o["picks"]["top"]["name"].as_str().unwrap(), "Shirt B");
    }

    #[test]
    fn every_pick_has_reasoning() {
        let o = build_outfit(&demo_wardrobe(), "work", 24.0, "sunny", &[]);
        for (slot, _) in o["picks"].as_object().unwrap() {
            assert!(o["reasons"][slot].is_array());
            assert!(!o["reasons"][slot].as_array().unwrap().is_empty());
        }
    }

    #[test]
    fn cost_per_wear_basic_math() {
        assert_eq!(cost_per_wear(&Item { price: 100.0, wears: 10, ..Item::default() }), Some(10.0));
        let cpw = cost_per_wear(&Item { price: 89.0, wears: 31, ..Item::default() }).unwrap();
        assert!((cpw - 2.87).abs() < 0.005);
    }

    #[test]
    fn never_worn_returns_none() {
        assert_eq!(cost_per_wear(&Item { price: 50.0, wears: 0, ..Item::default() }), None);
    }

    #[test]
    fn analytics_totals() {
        let w = demo_wardrobe();
        let a = analytics(&w);
        assert_eq!(a["total_items"].as_i64().unwrap(), 15);
        let total_value: f64 = w.iter().map(|i| i.price).sum();
        assert!((a["total_value"].as_f64().unwrap() - total_value).abs() < 0.01);
        let total_wears: i64 = w.iter().map(|i| i.wears).sum();
        assert_eq!(a["total_wears"].as_i64().unwrap(), total_wears);
        let cpws: Vec<f64> = a["roi_ranking"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["cost_per_wear"].as_f64().unwrap())
            .collect();
        let mut sorted = cpws.clone();
        sorted.sort_by(|x, y| x.partial_cmp(y).unwrap());
        assert_eq!(cpws, sorted);
        assert_eq!(a["roi_ranking"][0]["name"].as_str().unwrap(), "Navy Tee");
        assert_eq!(a["most_worn"][0]["name"].as_str().unwrap(), "Watch");
    }

    #[test]
    fn exact_size_and_matching_cut_scores_high() {
        let f = fit_score(
            &Item { size: "M".into(), fit_notes: "slim".into(), wears: 5, ..Item::default() },
            &profile(),
        );
        assert!(f["score"].as_i64().unwrap() >= 90);
    }

    #[test]
    fn wrong_size_scores_low() {
        let f = fit_score(
            &Item { size: "XL".into(), fit_notes: "slim".into(), wears: 5, ..Item::default() },
            &profile(),
        );
        assert!(f["score"].as_i64().unwrap() < 60);
    }

    #[test]
    fn cut_mismatch_costs_points() {
        let slim = fit_score(
            &Item { size: "M".into(), fit_notes: "slim".into(), wears: 5, ..Item::default() },
            &profile(),
        );
        let relaxed = fit_score(
            &Item { size: "M".into(), fit_notes: "relaxed".into(), wears: 5, ..Item::default() },
            &profile(),
        );
        assert!(slim["score"].as_i64().unwrap() > relaxed["score"].as_i64().unwrap());
    }

    #[test]
    fn return_risk_size_mismatch_is_high() {
        let r = return_risk("top", "XL", "Zara", 80.0, "navy", &demo_wardrobe(), &profile());
        assert!(r["risk"].as_i64().unwrap() >= 60);
        assert_eq!(r["band"].as_str().unwrap(), "High");
        assert!(r["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x.as_str().unwrap().contains("Size XL")));
    }

    #[test]
    fn return_risk_matching_size_is_low() {
        let r = return_risk("top", "M", "X", 15.0, "green", &demo_wardrobe(), &profile());
        assert!(r["risk"].as_i64().unwrap() < 30);
        assert_eq!(r["band"].as_str().unwrap(), "Low");
    }

    #[test]
    fn return_risk_unknown_brand_adds_risk() {
        let known = return_risk("top", "M", "X", 15.0, "green", &demo_wardrobe(), &profile());
        let unknown = return_risk("top", "M", "BrandNeverSeen", 15.0, "green", &demo_wardrobe(), &profile());
        assert!(unknown["risk"].as_i64().unwrap() > known["risk"].as_i64().unwrap());
    }

    #[test]
    fn perfect_wardrobe_scores_high() {
        let recent: HashSet<i64> = (1..=10).collect();
        let s = appearance_score(&demo_wardrobe(), &recent, 14, 14);
        assert_eq!(
            s["breakdown"]["wardrobe_completeness"]["points"].as_i64().unwrap(),
            30
        );
        assert_eq!(
            s["breakdown"]["rotation_variety"]["points"].as_i64().unwrap(),
            25
        );
        assert_eq!(
            s["breakdown"]["grooming_consistency"]["points"].as_i64().unwrap(),
            25
        );
        assert!(s["score"].as_i64().unwrap() >= 80);
    }

    #[test]
    fn empty_wardrobe_scores_zero() {
        let s = appearance_score(&[], &HashSet::new(), 0, 0);
        assert_eq!(s["score"].as_i64().unwrap(), 0);
    }

    #[test]
    fn breakdown_sums_to_total() {
        let recent: HashSet<i64> = [1, 2].iter().copied().collect();
        let s = appearance_score(&demo_wardrobe(), &recent, 3, 14);
        let total: i64 = s["breakdown"]
            .as_object()
            .unwrap()
            .values()
            .map(|v| v["points"].as_i64().unwrap())
            .sum();
        assert_eq!(total, s["score"].as_i64().unwrap());
    }

    #[test]
    fn only_owned_items_packed() {
        let w = demo_wardrobe();
        let p = build_packing(&w, &[], "Tokyo", 4, "sunny", &["casual".into(), "work".into()], "2026-11-11");
        let owned: HashSet<i64> = w.iter().map(|i| i.id).collect();
        for packed in p["packed"].as_array().unwrap() {
            if !packed["item_id"].is_null() {
                assert!(owned.contains(&packed["item_id"].as_i64().unwrap()));
            }
        }
    }

    #[test]
    fn tops_scale_with_days() {
        let w = demo_wardrobe();
        let p3 = build_packing(&w, &[], "Paris", 3, "sunny", &["casual".into()], "2026-11-11");
        let p6 = build_packing(&w, &[], "Paris", 6, "sunny", &["casual".into()], "2026-11-11");
        let tops3 = p3["packed"].as_array().unwrap().iter().filter(|x| x["category"] == "top").count();
        let tops6 = p6["packed"].as_array().unwrap().iter().filter(|x| x["category"] == "top").count();
        assert!(tops3 <= tops6);
    }

    #[test]
    fn rain_trip_packs_jacket() {
        let w = demo_wardrobe();
        let p = build_packing(&w, &[], "London", 3, "rain", &["casual".into()], "2026-11-11");
        let names: Vec<&str> = p["packed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"Rain Jacket"));
    }

    #[test]
    fn expired_grooming_not_packed() {
        let w = demo_wardrobe();
        let grooming = vec![
            GroomingProduct { id: 1, name: "Old Cream".into(), category: "skincare".into(), brand: "".into(), expiry_date: "2025-01-01".into(), routine: "morning".into(), notes: "".into() },
            GroomingProduct { id: 2, name: "Fresh Serum".into(), category: "skincare".into(), brand: "".into(), expiry_date: "2027-01-01".into(), routine: "morning".into(), notes: "".into() },
        ];
        let p = build_packing(&w, &grooming, "Bali", 2, "sunny", &["casual".into()], "2026-11-11");
        let names: Vec<&str> = p["packed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["name"].as_str().unwrap())
            .collect();
        assert!(!names.contains(&"Old Cream"));
        assert!(names.contains(&"Fresh Serum"));
    }

    #[test]
    fn no_similar_means_buy() {
        let r = shopping_check(&demo_wardrobe(), "top", "purple", 60.0);
        assert_eq!(r["verdict"].as_str().unwrap(), "BUY");
    }

    #[test]
    fn unworn_duplicate_means_skip() {
        let mut w = demo_wardrobe();
        w.push(Item { id: 99, name: "White Tee".into(), category: "top".into(), color: "white".into(), occasions: vec!["casual".into()], wears: 1, price: 20.0, ..Item::default() });
        let r = shopping_check(&w, "top", "white", 25.0);
        assert_eq!(r["verdict"].as_str().unwrap(), "SKIP");
    }

    #[test]
    fn heavily_worn_duplicate_means_buy() {
        let mut w = demo_wardrobe();
        w.push(Item { id: 99, name: "Old White Tee".into(), category: "top".into(), color: "white".into(), occasions: vec!["casual".into()], wears: 40, price: 20.0, ..Item::default() });
        let r = shopping_check(&w, "top", "white", 25.0);
        assert_eq!(r["verdict"].as_str().unwrap(), "BUY");
    }

    #[test]
    fn python_rounding_parity() {
        // Python round() is round-half-to-even; verify the helper matches.
        assert_eq!(round_half_even(2.5, 0), 2.0);
        assert_eq!(round_half_even(3.5, 0), 4.0);
        assert_eq!(round_half_even(0.625, 2), 0.62);
        assert_eq!(round_half_even(0.125, 2), 0.12);
        assert_eq!(round_half_even(2.675, 2), 2.67); // 2.675 is 2.67499… in binary
    }
}
