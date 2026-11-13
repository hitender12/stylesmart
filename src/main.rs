//! StyleSmart — full-stack app, Rust backend (axum + tokio + rusqlite).
//!
//! Run:  cargo run               (serves UI + JSON API on port 8000)
//!       cargo run -- --port 8001 --db /path/to/wardrobe.db

mod db;
mod engine;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use chrono::{Duration, Local};
use rusqlite::Connection;
use serde_json::{json, Value};

const DEMO_API_KEY: &str = "demo-key-1234";
const RATE_LIMIT: usize = 30; // requests per window per key
const RATE_WINDOW_SECS: u64 = 60;

#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<Connection>>,
    rate: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    static_dir: PathBuf,
}

fn today() -> String {
    Local::now().date_naive().format("%Y-%m-%d").to_string()
}

fn yesterday() -> String {
    (Local::now().date_naive() - Duration::days(1))
        .format("%Y-%m-%d")
        .to_string()
}

type ApiResult = (StatusCode, Json<Value>);

fn ok(v: Value) -> ApiResult {
    (StatusCode::OK, Json(v))
}

fn created(v: Value) -> ApiResult {
    (StatusCode::CREATED, Json(v))
}

fn err(status: StatusCode, msg: &str) -> ApiResult {
    (status, Json(json!({"error": msg})))
}

fn not_found() -> ApiResult {
    err(StatusCode::NOT_FOUND, "not found")
}

/// Parse a request body: empty body -> {}; invalid JSON -> 400.
fn parse_body(body: &Bytes) -> Result<Value, ApiResult> {
    if body.is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    match serde_json::from_slice::<Value>(body) {
        Ok(v) => Ok(v),
        Err(_) => Err(err(StatusCode::BAD_REQUEST, "invalid JSON")),
    }
}

fn parse_id(raw: &str) -> Option<i64> {
    raw.parse::<i64>().ok()
}

fn query_str<'a>(q: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    q.get(key).map(|s| s.as_str())
}

// ---------------------------------------------------------------- static

async fn index(State(state): State<AppState>) -> Response {
    serve_file(&state.static_dir, "index.html").await
}

async fn static_file(State(state): State<AppState>, Path(path): Path<String>) -> Response {
    if path.contains("..") {
        return not_found().into_response();
    }
    serve_file(&state.static_dir, &path).await
}

async fn serve_file(static_dir: &PathBuf, name: &str) -> Response {
    let full = static_dir.join(name);
    let ctype = match name.rsplit('.').next().unwrap_or("") {
        "html" => "text/html",
        "css" => "text/css",
        "js" => "application/javascript",
        _ => "application/octet-stream",
    };
    match tokio::fs::read(&full).await {
        Ok(data) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, ctype)],
            data,
        )
            .into_response(),
        Err(_) => not_found().into_response(),
    }
}

async fn fallback_404() -> ApiResult {
    not_found()
}

// ---------------------------------------------------------------- health

async fn health() -> ApiResult {
    ok(json!({"status": "ok", "app": "StyleSmart"}))
}

// ---------------------------------------------------------------- items

async fn list_items(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let conn = state.db.lock().unwrap();
    match db::list_items(
        &conn,
        query_str(&q, "category"),
        query_str(&q, "color"),
        query_str(&q, "search"),
    ) {
        Ok(items) => ok(json!(items)),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn get_item(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult {
    let Some(id) = parse_id(&id) else {
        return not_found();
    };
    let conn = state.db.lock().unwrap();
    match db::get_item(&conn, id) {
        Ok(Some(item)) => ok(json!(item)),
        Ok(None) => not_found(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

const CATEGORIES: [&str; 7] = [
    "top",
    "bottom",
    "footwear",
    "outerwear",
    "accessory",
    "grooming",
    "fragrance",
];

async fn create_item(State(state): State<AppState>, body: Bytes) -> ApiResult {
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e,
    };
    for f in ["name", "category"] {
        if db::get(&body, f).map(db::as_str).unwrap_or_default().is_empty() {
            return err(StatusCode::BAD_REQUEST, &format!("missing field: {}", f));
        }
    }
    let category = db::get(&body, "category").map(db::as_str).unwrap_or_default();
    if !CATEGORIES.contains(&category.as_str()) {
        return err(StatusCode::BAD_REQUEST, "unknown category");
    }
    let conn = state.db.lock().unwrap();
    match db::add_item(&conn, &body) {
        Ok(item) => created(json!(item)),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn update_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult {
    let Some(id) = parse_id(&id) else {
        return not_found();
    };
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let conn = state.db.lock().unwrap();
    match db::update_item(&conn, id, &body) {
        Ok(Some(item)) => ok(json!(item)),
        Ok(None) => not_found(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn delete_item(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult {
    let Some(id) = parse_id(&id) else {
        return not_found();
    };
    let conn = state.db.lock().unwrap();
    match db::delete_item(&conn, id) {
        Ok(true) => ok(json!({"deleted": id})),
        Ok(false) => not_found(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn log_wear(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult {
    let Some(id) = parse_id(&id) else {
        return not_found();
    };
    let conn = state.db.lock().unwrap();
    match db::log_wear(&conn, id, &today()) {
        Ok(Some(item)) => ok(json!(item)),
        Ok(None) => not_found(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

// ---------------------------------------------------------------- outfit

async fn get_outfit(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let occasion = query_str(&q, "occasion").unwrap_or("casual").to_string();
    if !engine::OCCASIONS.contains(&occasion.as_str()) {
        return err(
            StatusCode::BAD_REQUEST,
            &format!("unknown occasion: {}", occasion),
        );
    }
    let temp: f64 = query_str(&q, "temp")
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(22.0);
    let condition = query_str(&q, "condition").unwrap_or("sunny").to_string();
    if !engine::CONDITIONS.contains(&condition.as_str()) {
        return err(
            StatusCode::BAD_REQUEST,
            &format!("unknown condition: {}", condition),
        );
    }
    let conn = state.db.lock().unwrap();
    let yesterday_ids = match db::get_outfit_on(&conn, &yesterday()) {
        Ok(Some(o)) => o["item_ids"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_i64()).collect::<Vec<i64>>())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let items = match db::list_items(&conn, None, None, None) {
        Ok(items) => items,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    ok(engine::build_outfit(
        &items,
        &occasion,
        temp,
        &condition,
        &yesterday_ids,
    ))
}

async fn log_outfit(State(state): State<AppState>, body: Bytes) -> ApiResult {
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let ids = match db::get(&body, "item_ids") {
        Some(Value::Array(arr)) if !arr.is_empty() => arr
            .iter()
            .filter_map(|v| v.as_i64())
            .collect::<Vec<i64>>(),
        _ => return err(StatusCode::BAD_REQUEST, "item_ids list required"),
    };
    let occasion = db::get(&body, "occasion").map(db::as_str).unwrap_or_default();
    let temp: Option<f64> = db::get(&body, "temp").map(db::as_f64);
    let condition = db::get(&body, "condition")
        .map(db::as_str)
        .unwrap_or_default();
    let date = today();
    let conn = state.db.lock().unwrap();
    for i in &ids {
        let _ = db::log_wear(&conn, *i, &date);
    }
    if let Err(e) = db::record_outfit(&conn, &date, &ids, &occasion, temp, &condition) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    ok(json!({"logged": ids.len(), "date": date}))
}

async fn get_history(State(state): State<AppState>) -> ApiResult {
    let conn = state.db.lock().unwrap();
    match db::recent_outfits(&conn, 14) {
        Ok(rows) => ok(json!(rows)),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

// ---------------------------------------------------------------- analytics / score

async fn get_analytics(State(state): State<AppState>) -> ApiResult {
    let conn = state.db.lock().unwrap();
    match db::list_items(&conn, None, None, None) {
        Ok(items) => ok(engine::analytics(&items)),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn get_score(State(state): State<AppState>) -> ApiResult {
    let conn = state.db.lock().unwrap();
    let items = match db::list_items(&conn, None, None, None) {
        Ok(items) => items,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let recent = match db::recent_outfits(&conn, 14) {
        Ok(r) => r,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let mut recent_ids = std::collections::HashSet::new();
    for o in &recent {
        if let Some(arr) = o["item_ids"].as_array() {
            for v in arr {
                if let Some(id) = v.as_i64() {
                    recent_ids.insert(id);
                }
            }
        }
    }
    let start = (Local::now().date_naive() - Duration::days(6))
        .format("%Y-%m-%d")
        .to_string();
    let checks = match db::checks_in_range(&conn, &start, &today()) {
        Ok(c) => c,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let done = checks.iter().filter(|c| c["done"].as_bool().unwrap_or(false)).count() as i64;
    let expected = 7 * match db::list_grooming(&conn) {
        Ok(g) => g.len() as i64,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    ok(engine::appearance_score(
        &items,
        &recent_ids,
        done,
        expected,
    ))
}

// ---------------------------------------------------------------- fit

async fn get_fit_profile(State(state): State<AppState>) -> ApiResult {
    let conn = state.db.lock().unwrap();
    match db::get_profile(&conn) {
        Ok(p) => ok(json!(p)),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn update_fit_profile(State(state): State<AppState>, body: Bytes) -> ApiResult {
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let conn = state.db.lock().unwrap();
    match db::update_profile(&conn, &body) {
        Ok(p) => ok(json!(p)),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn fit_score(State(state): State<AppState>, body: Bytes) -> ApiResult {
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let item_id = db::get(&body, "item_id").map(db::as_i64).unwrap_or(0);
    let conn = state.db.lock().unwrap();
    let item = match db::get_item(&conn, item_id) {
        Ok(Some(item)) => item,
        Ok(None) => return err(StatusCode::NOT_FOUND, "item not found"),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let profile = match db::get_profile(&conn) {
        Ok(p) => p,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    ok(engine::fit_score(&item, &profile))
}

async fn return_risk(State(state): State<AppState>, body: Bytes) -> ApiResult {
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e,
    };
    for f in ["category", "size"] {
        if db::get(&body, f).map(db::as_str).unwrap_or_default().is_empty() {
            return err(StatusCode::BAD_REQUEST, &format!("missing field: {}", f));
        }
    }
    let category = db::get(&body, "category").map(db::as_str).unwrap_or_default();
    let size = db::get(&body, "size").map(db::as_str).unwrap_or_default();
    let brand = db::get(&body, "brand").map(db::as_str).unwrap_or_default();
    let price = db::get(&body, "price").map(db::as_f64).unwrap_or(0.0);
    let color = db::get(&body, "color").map(db::as_str).unwrap_or_default();
    let conn = state.db.lock().unwrap();
    let items = match db::list_items(&conn, None, None, None) {
        Ok(items) => items,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let profile = match db::get_profile(&conn) {
        Ok(p) => p,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    ok(engine::return_risk(
        &category, &size, &brand, price, &color, &items, &profile,
    ))
}

// ---------------------------------------------------------------- grooming

async fn get_grooming(State(state): State<AppState>) -> ApiResult {
    let date = today();
    let soon = (Local::now().date_naive() + Duration::days(30))
        .format("%Y-%m-%d")
        .to_string();
    let conn = state.db.lock().unwrap();
    let products = match db::list_grooming(&conn) {
        Ok(p) => p,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let checks = match db::get_checks(&conn, &date) {
        Ok(c) => c,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let mut out = Vec::new();
    for p in products {
        let status = if !p.expiry_date.is_empty() && p.expiry_date.as_str() < date.as_str() {
            "expired"
        } else if !p.expiry_date.is_empty() && p.expiry_date.as_str() <= soon.as_str() {
            "expiring_soon"
        } else {
            "ok"
        };
        let done_today = checks.get(&p.id.to_string()).copied().unwrap_or(false);
        let mut v = serde_json::to_value(&p).unwrap();
        v["expiry_status"] = json!(status);
        v["done_today"] = json!(done_today);
        out.push(v);
    }
    ok(json!({"date": date, "products": out}))
}

async fn create_grooming(State(state): State<AppState>, body: Bytes) -> ApiResult {
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e,
    };
    if db::get(&body, "name").map(db::as_str).unwrap_or_default().is_empty() {
        return err(StatusCode::BAD_REQUEST, "missing field: name");
    }
    let conn = state.db.lock().unwrap();
    match db::add_grooming(&conn, &body) {
        Ok(p) => created(json!(p)),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn delete_grooming(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult {
    let Some(id) = parse_id(&id) else {
        return not_found();
    };
    let conn = state.db.lock().unwrap();
    match db::delete_grooming(&conn, id) {
        Ok(true) => ok(json!({"deleted": id})),
        Ok(false) => not_found(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn grooming_check(State(state): State<AppState>, body: Bytes) -> ApiResult {
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let date = db::get(&body, "date")
        .map(db::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(today);
    let product_id = db::get(&body, "product_id").map(db::as_i64).unwrap_or(0);
    let done = db::get(&body, "done").map(db::as_bool).unwrap_or(true);
    let conn = state.db.lock().unwrap();
    if let Err(e) = db::set_check(&conn, &date, product_id, done) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    match db::get_checks(&conn, &date) {
        Ok(checks) => ok(json!({"date": date, "checks": checks})),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn grooming_checklist(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let date = query_str(&q, "date")
        .map(|s| s.to_string())
        .unwrap_or_else(today);
    let conn = state.db.lock().unwrap();
    match db::get_checks(&conn, &date) {
        Ok(checks) => ok(json!({"date": date, "checks": checks})),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

// ---------------------------------------------------------------- packing / shopping

async fn create_packing(State(state): State<AppState>, body: Bytes) -> ApiResult {
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e,
    };
    for f in ["destination", "days"] {
        // Python parity: `if not body.get(f)` — missing, null, "", or 0.
        let missing = match db::get(&body, f) {
            None | Some(Value::Null) => true,
            Some(v) => {
                if f == "days" {
                    db::as_i64(v) == 0
                } else {
                    db::as_str(v).is_empty()
                }
            }
        };
        if missing {
            return err(StatusCode::BAD_REQUEST, &format!("missing field: {}", f));
        }
    }
    let days = db::get(&body, "days").map(db::as_i64).unwrap_or(0);
    if days < 1 || days > 30 {
        return err(StatusCode::BAD_REQUEST, "days must be 1-30");
    }
    let destination = db::get(&body, "destination").map(db::as_str).unwrap_or_default();
    let weather = db::get(&body, "weather").map(db::as_str).unwrap_or_default();
    let occasions: Vec<String> = match db::get(&body, "occasions") {
        Some(v) => db::as_occasions(v),
        None => vec!["casual".to_string()],
    };
    let conn = state.db.lock().unwrap();
    let items = match db::list_items(&conn, None, None, None) {
        Ok(items) => items,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let grooming = match db::list_grooming(&conn) {
        Ok(g) => g,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let packing = engine::build_packing(
        &items,
        &grooming,
        &destination,
        days,
        &weather,
        &occasions,
        &today(),
    );
    let saved = match db::save_packing(&conn, &body, &packing["packed"]) {
        Ok(s) => s,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    // The frontend reads `items` on the generated packing response while the
    // documented API field is `packed`; return both (same array) so the
    // existing frontend works unchanged.
    let mut out = packing;
    out["items"] = out["packed"].clone();
    out["id"] = saved["id"].clone();
    ok(out)
}

async fn list_packings(State(state): State<AppState>) -> ApiResult {
    let conn = state.db.lock().unwrap();
    match db::list_packings(&conn) {
        Ok(rows) => ok(json!(rows)),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn shopping_check(State(state): State<AppState>, body: Bytes) -> ApiResult {
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e,
    };
    for f in ["category", "color"] {
        if db::get(&body, f).map(db::as_str).unwrap_or_default().is_empty() {
            return err(StatusCode::BAD_REQUEST, &format!("missing field: {}", f));
        }
    }
    let category = db::get(&body, "category").map(db::as_str).unwrap_or_default();
    let color = db::get(&body, "color").map(db::as_str).unwrap_or_default();
    let price = db::get(&body, "price").map(db::as_f64).unwrap_or(0.0);
    let conn = state.db.lock().unwrap();
    let items = match db::list_items(&conn, None, None, None) {
        Ok(items) => items,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    ok(engine::shopping_check(&items, &category, &color, price))
}

// ---------------------------------------------------------------- B2B API v1

fn v1_auth_and_rate(state: &AppState, headers: &HeaderMap) -> Result<(), ApiResult> {
    let key = headers
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if key != DEMO_API_KEY {
        return Err(err(
            StatusCode::UNAUTHORIZED,
            "unauthorized — send a valid X-API-Key header",
        ));
    }
    let now = Instant::now();
    let mut buckets = state.rate.lock().unwrap();
    let bucket = buckets.entry(key.to_string()).or_default();
    bucket.retain(|t| now.duration_since(*t).as_secs() < RATE_WINDOW_SECS);
    if bucket.len() >= RATE_LIMIT {
        return Err(err(
            StatusCode::TOO_MANY_REQUESTS,
            "rate limit exceeded — 30 requests per minute",
        ));
    }
    bucket.push(now);
    Ok(())
}

async fn v1_fit_predict(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult {
    if let Err(e) = v1_auth_and_rate(&state, &headers) {
        return e;
    }
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let category = db::get(&body, "category").map(db::as_str).unwrap_or_default();
    let size = db::get(&body, "size").map(db::as_str).unwrap_or_default();
    if category.is_empty() || size.is_empty() {
        return err(StatusCode::BAD_REQUEST, "category and size required");
    }
    let brand = db::get(&body, "brand").map(db::as_str).unwrap_or_default();
    let price = db::get(&body, "price").map(db::as_f64).unwrap_or(0.0);
    let color = db::get(&body, "color").map(db::as_str).unwrap_or_default();
    let conn = state.db.lock().unwrap();
    let items = match db::list_items(&conn, None, None, None) {
        Ok(items) => items,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let profile = match db::get_profile(&conn) {
        Ok(p) => p,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let risk = engine::return_risk(&category, &size, &brand, price, &color, &items, &profile);
    ok(json!({
        "category": category,
        "size": size,
        "return_risk": risk["risk"],
        "band": risk["band"],
        "reasons": risk["reasons"],
    }))
}

async fn v1_recommend(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult {
    if let Err(e) = v1_auth_and_rate(&state, &headers) {
        return e;
    }
    let body = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let occasion = db::get(&body, "occasion")
        .map(db::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "casual".to_string());
    if !engine::OCCASIONS.contains(&occasion.as_str()) {
        return err(
            StatusCode::BAD_REQUEST,
            &format!("unknown occasion: {}", occasion),
        );
    }
    let temp = db::get(&body, "temp").map(db::as_f64).unwrap_or(22.0);
    let condition = db::get(&body, "condition")
        .map(db::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "sunny".to_string());
    let conn = state.db.lock().unwrap();
    let yesterday_ids = match db::get_outfit_on(&conn, &yesterday()) {
        Ok(Some(o)) => o["item_ids"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_i64()).collect::<Vec<i64>>())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let items = match db::list_items(&conn, None, None, None) {
        Ok(items) => items,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    ok(engine::build_outfit(
        &items,
        &occasion,
        temp,
        &condition,
        &yesterday_ids,
    ))
}

// ---------------------------------------------------------------- main

fn parse_args() -> (String, u16, String) {
    let args: Vec<String> = std::env::args().collect();
    let mut host = "127.0.0.1".to_string();
    let mut port: u16 = 8000;
    let mut db = "wardrobe.db".to_string();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                if i + 1 < args.len() {
                    port = args[i + 1].parse().unwrap_or(8000);
                    i += 1;
                }
            }
            "--db" => {
                if i + 1 < args.len() {
                    db = args[i + 1].clone();
                    i += 1;
                }
            }
            "--host" => {
                if i + 1 < args.len() {
                    host = args[i + 1].clone();
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    (host, port, db)
}

#[tokio::main]
async fn main() {
    let (host, port, db_path) = parse_args();
    let conn = db::init_db(&db_path).expect("failed to initialise database");
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let static_dir = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("static");
    let static_dir = if static_dir.join("index.html").exists() {
        static_dir
    } else if let Some(d) = exe_dir {
        // Fall back to <exe-dir>/../static (installed layout).
        d.join("..").join("static")
    } else {
        static_dir
    };
    let state = AppState {
        db: Arc::new(Mutex::new(conn)),
        rate: Arc::new(Mutex::new(HashMap::new())),
        static_dir,
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/index.html", get(index))
        .route("/static/*path", get(static_file))
        .route("/api/health", get(health))
        .route("/api/items", get(list_items).post(create_item))
        .route("/api/items/:id", get(get_item).put(update_item).delete(delete_item))
        .route("/api/items/:id/wear", post(log_wear))
        .route("/api/outfit", get(get_outfit))
        .route("/api/outfit/log", post(log_outfit))
        .route("/api/history", get(get_history))
        .route("/api/analytics", get(get_analytics))
        .route("/api/score", get(get_score))
        .route("/api/fit-profile", get(get_fit_profile).put(update_fit_profile))
        .route("/api/fit-score", post(fit_score))
        .route("/api/return-risk", post(return_risk))
        .route("/api/grooming", get(get_grooming).post(create_grooming))
        .route("/api/grooming/:id", delete(delete_grooming))
        .route("/api/grooming/check", post(grooming_check))
        .route("/api/grooming/checklist", get(grooming_checklist))
        .route("/api/packing", post(create_packing))
        .route("/api/packings", get(list_packings))
        .route("/api/shopping-check", post(shopping_check))
        .route("/api/v1/fit-predict", post(v1_fit_predict))
        .route("/api/v1/recommend", post(v1_recommend))
        .fallback(fallback_404)
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", host, port).parse().expect("invalid bind address");
    println!("StyleSmart running at http://{}:{}  (db: {})", host, port, db_path);
    println!("Demo B2B API key: demo-key-1234  (header X-API-Key)");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("failed to bind");
    axum::serve(listener, app).await.expect("server error");
}
