//! StyleSmart integration tests — live HTTP API against a spawned server
//! binary on an ephemeral port with a fresh seeded database per test.
//! (Rust port of the Python TestAPI suite.)

use serde_json::{json, Value};
use std::io::Read;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Duration;

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct TestServer {
    child: Child,
    port: u16,
    db_path: PathBuf,
}

impl TestServer {
    fn start() -> Self {
        let port = free_port();
        let db_path = std::env::temp_dir().join(format!(
            "stylesmart_api_test_{}_{}.db",
            std::process::id(),
            port
        ));
        let _ = std::fs::remove_file(&db_path);
        let child = Command::new(env!("CARGO_BIN_EXE_stylesmart"))
            .args([
                "--port",
                &port.to_string(),
                "--db",
                db_path.to_str().unwrap(),
                "--host",
                "127.0.0.1",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("failed to spawn stylesmart server");
        let server = TestServer { child, port, db_path };
        // Wait for /api/health (up to 10 s).
        let url = format!("http://127.0.0.1:{}/api/health", port);
        for _ in 0..100 {
            if let Ok(resp) = ureq::get(&url).timeout(Duration::from_secs(1)).call() {
                if resp.status() == 200 {
                    return server;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("server did not come up on port {}", port);
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }

    /// Returns (status, parsed JSON body).
    fn req(&self, method: &str, path: &str, body: Option<Value>, key: Option<&str>) -> (u16, Value) {
        let url = self.url(path);
        let mut req = match method {
            "GET" => ureq::get(&url),
            "POST" => ureq::post(&url),
            "PUT" => ureq::put(&url),
            "DELETE" => ureq::delete(&url),
            _ => panic!("bad method"),
        };
        req = req.set("Content-Type", "application/json");
        if let Some(k) = key {
            req = req.set("X-API-Key", k);
        }
        let result = match body {
            Some(b) => req.send_json(b),
            None => req.call(),
        };
        match result {
            Ok(resp) => (resp.status(), read_json(resp)),
            Err(ureq::Error::Status(status, resp)) => (status, read_json(resp)),
            Err(e) => panic!("request failed: {}", e),
        }
    }

    fn get(&self, path: &str) -> (u16, Value) {
        self.req("GET", path, None, None)
    }
}

fn read_json(resp: ureq::Response) -> Value {
    let mut body = String::new();
    let _ = resp.into_reader().read_to_string(&mut body);
    serde_json::from_str(&body).unwrap_or(Value::Null)
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.db_path);
    }
}

const DEMO_KEY: &str = "demo-key-1234";

#[test]
fn health() {
    let s = TestServer::start();
    let (status, body) = s.get("/api/health");
    assert_eq!(status, 200);
    assert_eq!(body["status"], "ok");
}

#[test]
fn items_list_and_create() {
    let s = TestServer::start();
    let (status, items) = s.get("/api/items");
    assert_eq!(status, 200);
    assert!(items.as_array().unwrap().len() >= 24);
    let (status, created) = s.req(
        "POST",
        "/api/items",
        Some(json!({"name": "API Test Scarf", "category": "accessory",
                    "color": "red", "price": 29, "size": "M"})),
        None,
    );
    assert_eq!(status, 201);
    assert_eq!(created["name"], "API Test Scarf");
    let (status, _) = s.req("DELETE", &format!("/api/items/{}", created["id"]), None, None);
    assert_eq!(status, 200);
}

#[test]
fn items_validation() {
    let s = TestServer::start();
    let (status, _) = s.req("POST", "/api/items", Some(json!({"category": "top"})), None);
    assert_eq!(status, 400);
    let (status, _) = s.req(
        "POST",
        "/api/items",
        Some(json!({"name": "X", "category": "nope"})),
        None,
    );
    assert_eq!(status, 400);
}

#[test]
fn outfit_all_occasions() {
    let s = TestServer::start();
    for occ in ["work", "casual", "formal", "date", "gym", "travel"] {
        let (status, o) = s.get(&format!("/api/outfit?occasion={}&temp=22&condition=sunny", occ));
        assert_eq!(status, 200, "occasion {}", occ);
        assert!(o["picks"]["top"].is_object(), "occasion {}", occ);
        assert!(o["picks"]["bottom"].is_object(), "occasion {}", occ);
    }
}

#[test]
fn outfit_bad_occasion() {
    let s = TestServer::start();
    let (status, _) = s.get("/api/outfit?occasion=party&temp=22&condition=sunny");
    assert_eq!(status, 400);
}

#[test]
fn outfit_log_updates_wears() {
    let s = TestServer::start();
    let (_, o) = s.get("/api/outfit?occasion=casual&temp=22&condition=sunny");
    let ids: Vec<i64> = o["picks"]
        .as_object()
        .unwrap()
        .values()
        .map(|v| v["id"].as_i64().unwrap())
        .collect();
    let (_, items_before) = s.get("/api/items");
    let before: std::collections::HashMap<i64, i64> = items_before
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["id"].as_i64().unwrap(), i["wears"].as_i64().unwrap()))
        .collect();
    let (status, _) = s.req(
        "POST",
        "/api/outfit/log",
        Some(json!({"item_ids": ids, "occasion": "casual", "temp": 22, "condition": "sunny"})),
        None,
    );
    assert_eq!(status, 200);
    let (_, items_after) = s.get("/api/items");
    let after: std::collections::HashMap<i64, i64> = items_after
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["id"].as_i64().unwrap(), i["wears"].as_i64().unwrap()))
        .collect();
    for i in &ids {
        assert_eq!(after[i], before[i] + 1);
    }
}

#[test]
fn analytics() {
    let s = TestServer::start();
    let (status, a) = s.get("/api/analytics");
    assert_eq!(status, 200);
    // Pinned to the seed data (24 items) — these are the documented values.
    assert_eq!(a["total_items"].as_i64().unwrap(), 24);
    assert_eq!(a["total_value"].as_f64().unwrap(), 2453.0);
    assert_eq!(a["total_wears"].as_i64().unwrap(), 301);
    // Mean over ALL items, unworn counted at full price.
    assert_eq!(a["average_cost_per_wear"].as_f64().unwrap(), 12.42);
    assert!(a["roi_ranking"].is_array());
    // Cheapest cost-per-wear first.
    assert_eq!(
        a["roi_ranking"][0]["name"].as_str().unwrap(),
        "Navy Crew T-Shirt"
    );
}

#[test]
fn score() {
    let s = TestServer::start();
    let (status, sc) = s.get("/api/score");
    assert_eq!(status, 200);
    let score = sc["score"].as_i64().unwrap();
    assert!((0..=100).contains(&score));
    let total: i64 = sc["breakdown"]
        .as_object()
        .unwrap()
        .values()
        .map(|v| v["points"].as_i64().unwrap())
        .sum();
    assert_eq!(total, score);
}

#[test]
fn fit_profile_roundtrip() {
    let s = TestServer::start();
    let (status, _) = s.get("/api/fit-profile");
    assert_eq!(status, 200);
    let (status, p2) = s.req(
        "PUT",
        "/api/fit-profile",
        Some(json!({"fit_preference": "regular"})),
        None,
    );
    assert_eq!(status, 200);
    assert_eq!(p2["fit_preference"], "regular");
    let _ = s.req(
        "PUT",
        "/api/fit-profile",
        Some(json!({"fit_preference": "slim"})),
        None,
    );
}

#[test]
fn return_risk_endpoint() {
    let s = TestServer::start();
    let (status, r) = s.req(
        "POST",
        "/api/return-risk",
        Some(json!({"category": "footwear", "size": "11",
                    "brand": "Unknown", "price": 200, "color": "red"})),
        None,
    );
    assert_eq!(status, 200);
    assert!(r["risk"].is_number());
}

#[test]
fn grooming_checklist_persists() {
    let s = TestServer::start();
    let (status, g) = s.get("/api/grooming");
    assert_eq!(status, 200);
    let pid = g["products"][0]["id"].as_i64().unwrap();
    let (status, body) = s.req(
        "POST",
        "/api/grooming/check",
        Some(json!({"product_id": pid, "done": true})),
        None,
    );
    assert_eq!(status, 200);
    assert_eq!(body["checks"][pid.to_string()], true);
    let (_, g2) = s.get("/api/grooming");
    let p = g2["products"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"].as_i64().unwrap() == pid)
        .unwrap();
    assert_eq!(p["done_today"], true);
}

#[test]
fn packing_only_owned_items() {
    let s = TestServer::start();
    let (_, items) = s.get("/api/items");
    let owned: std::collections::HashSet<i64> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_i64().unwrap())
        .collect();
    let (status, p) = s.req(
        "POST",
        "/api/packing",
        Some(json!({"destination": "Test City", "days": 3,
                    "weather": "sunny", "occasions": ["casual"]})),
        None,
    );
    assert_eq!(status, 200);
    for packed in p["packed"].as_array().unwrap() {
        if !packed["item_id"].is_null() {
            assert!(owned.contains(&packed["item_id"].as_i64().unwrap()));
        }
    }
    // The frontend reads `items` on the generated response; the alias must exist.
    assert!(p["items"].is_array());
    assert_eq!(p["items"], p["packed"]);
}

#[test]
fn shopping_check_endpoint() {
    let s = TestServer::start();
    let (status, r) = s.req(
        "POST",
        "/api/shopping-check",
        Some(json!({"category": "top", "color": "purple", "price": 60})),
        None,
    );
    assert_eq!(status, 200);
    assert_eq!(r["verdict"], "BUY");
}

#[test]
fn v1_requires_api_key() {
    let s = TestServer::start();
    let (status, _) = s.req(
        "POST",
        "/api/v1/fit-predict",
        Some(json!({"category": "top", "size": "M"})),
        None,
    );
    assert_eq!(status, 401);
}

#[test]
fn v1_fit_predict_with_key() {
    let s = TestServer::start();
    let (status, r) = s.req(
        "POST",
        "/api/v1/fit-predict",
        Some(json!({"category": "top", "size": "M"})),
        Some(DEMO_KEY),
    );
    assert_eq!(status, 200);
    assert!(r["return_risk"].is_number());
}

#[test]
fn v1_recommend_with_key() {
    let s = TestServer::start();
    let (status, r) = s.req(
        "POST",
        "/api/v1/recommend",
        Some(json!({"occasion": "work", "temp": 24, "condition": "sunny"})),
        Some(DEMO_KEY),
    );
    assert_eq!(status, 200);
    assert!(r["picks"]["top"].is_object());
}

#[test]
fn static_ui_serves() {
    let s = TestServer::start();
    let url = s.url("/");
    let resp = ureq::get(&url).call().unwrap();
    assert_eq!(resp.status(), 200);
    let mut body = String::new();
    resp.into_reader().read_to_string(&mut body).unwrap();
    assert!(body.contains("StyleSmart"));
    assert!(body.contains("app.js"));
}
