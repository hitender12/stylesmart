//! StyleSmart — SQLite persistence layer (Rust port of db.py).
//!
//! Tables:
//!   items           wardrobe items
//!   fit_profile     single-row user fit profile (id = 1)
//!   grooming        grooming / skincare products
//!   routine_checks  daily routine checklist (date, product_id, done)
//!   outfit_history  outfits the user actually wore (drives the no-repeat rule)
//!   packings        saved travel packing lists

use std::collections::BTreeMap;
use std::collections::HashMap;

use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    category TEXT NOT NULL,
    color TEXT NOT NULL,
    color_hex TEXT NOT NULL DEFAULT '#888888',
    brand TEXT NOT NULL DEFAULT '',
    price REAL NOT NULL DEFAULT 0,
    purchase_date TEXT NOT NULL DEFAULT '',
    wears INTEGER NOT NULL DEFAULT 0,
    last_worn TEXT,
    size TEXT NOT NULL DEFAULT '',
    fit_notes TEXT NOT NULL DEFAULT '',
    occasions TEXT NOT NULL DEFAULT 'casual',
    material TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS fit_profile (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    sizes TEXT NOT NULL DEFAULT '{}',
    fit_preference TEXT NOT NULL DEFAULT 'regular',
    body_notes TEXT NOT NULL DEFAULT '',
    height_cm INTEGER,
    weight_kg REAL
);
CREATE TABLE IF NOT EXISTS grooming (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    category TEXT NOT NULL,
    brand TEXT NOT NULL DEFAULT '',
    expiry_date TEXT NOT NULL DEFAULT '',
    routine TEXT NOT NULL DEFAULT 'morning',
    notes TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS routine_checks (
    date TEXT NOT NULL,
    product_id INTEGER NOT NULL,
    done INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (date, product_id)
);
CREATE TABLE IF NOT EXISTS outfit_history (
    date TEXT PRIMARY KEY,
    item_ids TEXT NOT NULL DEFAULT '[]',
    occasion TEXT NOT NULL DEFAULT '',
    temp REAL,
    condition TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS packings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    destination TEXT NOT NULL,
    days INTEGER NOT NULL,
    weather TEXT NOT NULL DEFAULT '',
    occasions TEXT NOT NULL DEFAULT '',
    items TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT ''
);
";

// ---------------------------------------------------------------- types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: i64,
    pub name: String,
    pub category: String,
    pub color: String,
    pub color_hex: String,
    pub brand: String,
    pub price: f64,
    pub purchase_date: String,
    pub wears: i64,
    pub last_worn: Option<String>,
    pub size: String,
    pub fit_notes: String,
    pub occasions: Vec<String>,
    pub material: String,
    pub created_at: String,
}

impl Default for Item {
    fn default() -> Self {
        Item {
            id: 0,
            name: "Test".to_string(),
            category: "top".to_string(),
            color: "white".to_string(),
            color_hex: "#fff".to_string(),
            brand: "X".to_string(),
            price: 50.0,
            purchase_date: String::new(),
            wears: 0,
            last_worn: None,
            size: "M".to_string(),
            fit_notes: "regular".to_string(),
            occasions: vec!["casual".to_string(), "work".to_string()],
            material: "cotton".to_string(),
            created_at: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    pub sizes: BTreeMap<String, String>,
    pub fit_preference: String,
    pub body_notes: String,
    pub height_cm: Option<i64>,
    pub weight_kg: Option<f64>,
}

impl Default for Profile {
    fn default() -> Self {
        let mut sizes = BTreeMap::new();
        sizes.insert("top".to_string(), "M".to_string());
        sizes.insert("bottom".to_string(), "32".to_string());
        sizes.insert("footwear".to_string(), "9".to_string());
        sizes.insert("outerwear".to_string(), "M".to_string());
        sizes.insert("accessory".to_string(), "M".to_string());
        Profile {
            id: None,
            sizes,
            fit_preference: "slim".to_string(),
            body_notes: "Athletic build, broad shoulders, prefers tapered trousers."
                .to_string(),
            height_cm: Some(178),
            weight_kg: Some(76.0),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroomingProduct {
    pub id: i64,
    pub name: String,
    pub category: String,
    pub brand: String,
    pub expiry_date: String,
    pub routine: String,
    pub notes: String,
}

// ---------------------------------------------------------------- value coercion helpers
// (mirrors Python's loose float()/int() coercion of JSON fields)

pub fn as_f64(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::String(s) => s.trim().parse::<f64>().unwrap_or(0.0),
        Value::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        _ => 0.0,
    }
}

pub fn as_i64(v: &Value) -> i64 {
    match v {
        Value::Number(n) => n.as_i64().unwrap_or_else(|| n.as_f64().unwrap_or(0.0) as i64),
        Value::String(s) => s.trim().parse::<i64>().unwrap_or(0),
        Value::Bool(b) => {
            if *b {
                1
            } else {
                0
            }
        }
        _ => 0,
    }
}

pub fn as_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

pub fn as_bool(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        Value::String(s) => !s.is_empty() && s != "false" && s != "0",
        _ => false,
    }
}

/// Occasions may arrive as a JSON list or as a CSV string (both accepted).
pub fn as_occasions(v: &Value) -> Vec<String> {
    match v {
        Value::Array(arr) => arr.iter().map(as_str).filter(|s| !s.is_empty()).collect(),
        Value::String(s) => s
            .split(',')
            .map(|p| p.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

pub fn get<'a>(body: &'a Value, key: &str) -> Option<&'a Value> {
    body.as_object().and_then(|o| o.get(key))
}

// ---------------------------------------------------------------- connection / seed

pub fn connect(db_path: &str) -> rusqlite::Result<Connection> {
    let conn = Connection::open(db_path)?;
    conn.execute_batch("PRAGMA foreign_keys = ON")?;
    Ok(conn)
}

pub fn init_db(db_path: &str) -> rusqlite::Result<Connection> {
    let conn = connect(db_path)?;
    conn.execute_batch(SCHEMA)?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))?;
    if count == 0 {
        seed(&conn)?;
    }
    Ok(conn)
}

struct SeedItem {
    name: &'static str,
    category: &'static str,
    color: &'static str,
    hex: &'static str,
    brand: &'static str,
    price: f64,
    purchase_date: &'static str,
    wears: i64,
    last_worn: Option<&'static str>,
    size: &'static str,
    fit_notes: &'static str,
    occasions: &'static str,
    material: &'static str,
}

const SEED_ITEMS: &[SeedItem] = &[
    SeedItem { name: "White Oxford Shirt", category: "top", color: "white", hex: "#F5F5F5", brand: "Charles Tyrwhitt", price: 79.0, purchase_date: "2026-01-12", wears: 14, last_worn: Some("2026-11-09"), size: "M", fit_notes: "slim", occasions: "work,formal,date", material: "cotton" },
    SeedItem { name: "Navy Crew T-Shirt", category: "top", color: "navy", hex: "#1F2A44", brand: "Uniqlo", price: 15.0, purchase_date: "2026-03-02", wears: 22, last_worn: Some("2026-11-10"), size: "M", fit_notes: "regular", occasions: "casual,gym,travel", material: "cotton" },
    SeedItem { name: "Grey Merino Sweater", category: "top", color: "grey", hex: "#8A8D91", brand: "Massimo Dutti", price: 89.0, purchase_date: "2025-11-20", wears: 9, last_worn: Some("2026-11-06"), size: "M", fit_notes: "regular", occasions: "work,casual", material: "merino wool" },
    SeedItem { name: "Black Slim Polo", category: "top", color: "black", hex: "#1A1A1A", brand: "Lacoste", price: 95.0, purchase_date: "2026-02-14", wears: 11, last_worn: Some("2026-11-08"), size: "M", fit_notes: "slim", occasions: "work,casual,date", material: "cotton pique" },
    SeedItem { name: "Blue Linen Shirt", category: "top", color: "blue", hex: "#5B7FA6", brand: "H&M", price: 35.0, purchase_date: "2026-05-30", wears: 7, last_worn: Some("2026-11-05"), size: "M", fit_notes: "relaxed", occasions: "casual,date,travel", material: "linen" },
    SeedItem { name: "Navy Chinos", category: "bottom", color: "navy", hex: "#1F2A44", brand: "Dockers", price: 59.0, purchase_date: "2026-01-20", wears: 16, last_worn: Some("2026-11-09"), size: "32", fit_notes: "slim", occasions: "work,casual,date", material: "cotton twill" },
    SeedItem { name: "Dark Indigo Jeans", category: "bottom", color: "blue", hex: "#2E4057", brand: "Levi's", price: 89.0, purchase_date: "2025-09-11", wears: 31, last_worn: Some("2026-11-10"), size: "32", fit_notes: "regular", occasions: "casual,date,travel", material: "denim" },
    SeedItem { name: "Black Formal Trousers", category: "bottom", color: "black", hex: "#1A1A1A", brand: "Hugo Boss", price: 149.0, purchase_date: "2025-12-05", wears: 8, last_worn: Some("2026-11-04"), size: "32", fit_notes: "slim", occasions: "work,formal", material: "wool" },
    SeedItem { name: "Grey Shorts", category: "bottom", color: "grey", hex: "#8A8D91", brand: "Nike", price: 45.0, purchase_date: "2026-06-15", wears: 12, last_worn: Some("2026-11-07"), size: "M", fit_notes: "relaxed", occasions: "casual,gym,travel", material: "polyester" },
    SeedItem { name: "Brown Leather Oxfords", category: "footwear", color: "brown", hex: "#6B4226", brand: "Clarks", price: 129.0, purchase_date: "2025-10-02", wears: 18, last_worn: Some("2026-11-09"), size: "9", fit_notes: "regular", occasions: "work,formal,date", material: "leather" },
    SeedItem { name: "White Sneakers", category: "footwear", color: "white", hex: "#F5F5F5", brand: "Adidas", price: 110.0, purchase_date: "2026-04-18", wears: 26, last_worn: Some("2026-11-10"), size: "9", fit_notes: "regular", occasions: "casual,travel,gym", material: "leather" },
    SeedItem { name: "Black Running Shoes", category: "footwear", color: "black", hex: "#1A1A1A", brand: "Nike", price: 139.0, purchase_date: "2026-02-01", wears: 20, last_worn: Some("2026-11-08"), size: "9", fit_notes: "regular", occasions: "gym,casual", material: "mesh" },
    SeedItem { name: "Tan Suede Loafers", category: "footwear", color: "brown", hex: "#B08954", brand: "Loake", price: 159.0, purchase_date: "2026-03-25", wears: 6, last_worn: Some("2026-11-03"), size: "9", fit_notes: "regular", occasions: "casual,date", material: "suede" },
    SeedItem { name: "Navy Blazer", category: "outerwear", color: "navy", hex: "#1F2A44", brand: "Massimo Dutti", price: 199.0, purchase_date: "2025-11-15", wears: 10, last_worn: Some("2026-11-09"), size: "M", fit_notes: "slim", occasions: "work,formal,date", material: "wool" },
    SeedItem { name: "Black Rain Jacket", category: "outerwear", color: "black", hex: "#1A1A1A", brand: "Uniqlo", price: 69.0, purchase_date: "2026-05-10", wears: 4, last_worn: Some("2026-10-28"), size: "M", fit_notes: "regular", occasions: "casual,travel,work", material: "nylon" },
    SeedItem { name: "Camel Wool Coat", category: "outerwear", color: "beige", hex: "#C8A97E", brand: "COS", price: 249.0, purchase_date: "2025-12-12", wears: 5, last_worn: Some("2026-11-06"), size: "M", fit_notes: "regular", occasions: "work,formal,date", material: "wool" },
    SeedItem { name: "Brown Leather Belt", category: "accessory", color: "brown", hex: "#6B4226", brand: "Hugo Boss", price: 79.0, purchase_date: "2025-10-02", wears: 18, last_worn: Some("2026-11-09"), size: "M", fit_notes: "regular", occasions: "work,formal,casual,date", material: "leather" },
    SeedItem { name: "Silver Watch", category: "accessory", color: "grey", hex: "#C0C0C0", brand: "Seiko", price: 299.0, purchase_date: "2025-08-19", wears: 40, last_worn: Some("2026-11-10"), size: "M", fit_notes: "regular", occasions: "work,formal,casual,date,travel", material: "steel" },
    SeedItem { name: "Navy Silk Tie", category: "accessory", color: "navy", hex: "#1F2A44", brand: "Charles Tyrwhitt", price: 49.0, purchase_date: "2026-01-12", wears: 5, last_worn: Some("2026-11-04"), size: "M", fit_notes: "regular", occasions: "work,formal", material: "silk" },
    SeedItem { name: "Vitamin C Serum", category: "grooming", color: "orange", hex: "#E8912D", brand: "The Ordinary", price: 12.0, purchase_date: "2026-09-01", wears: 0, last_worn: None, size: "", fit_notes: "regular", occasions: "casual", material: "" },
    SeedItem { name: "Moisturiser SPF 30", category: "grooming", color: "white", hex: "#F5F5F5", brand: "CeraVe", price: 18.0, purchase_date: "2026-08-15", wears: 0, last_worn: None, size: "", fit_notes: "regular", occasions: "casual", material: "" },
    SeedItem { name: "Beard Oil", category: "grooming", color: "brown", hex: "#6B4226", brand: "Beardbrand", price: 24.0, purchase_date: "2026-10-01", wears: 0, last_worn: None, size: "", fit_notes: "regular", occasions: "casual", material: "" },
    SeedItem { name: "Citrus Day EDT", category: "fragrance", color: "yellow", hex: "#E8C547", brand: "Acqua di Parma", price: 98.0, purchase_date: "2026-04-22", wears: 12, last_worn: Some("2026-11-10"), size: "", fit_notes: "regular", occasions: "casual,date,work,travel", material: "" },
    SeedItem { name: "Woody Evening EDP", category: "fragrance", color: "black", hex: "#1A1A1A", brand: "Tom Ford", price: 165.0, purchase_date: "2025-12-20", wears: 7, last_worn: Some("2026-11-07"), size: "", fit_notes: "regular", occasions: "date,formal,work", material: "" },
];

const SEED_GROOMING: &[(&str, &str, &str, &str, &str, &str)] = &[
    ("Vitamin C Serum", "skincare", "The Ordinary", "2027-03-01", "morning", "Apply before moisturiser"),
    ("Moisturiser SPF 30", "skincare", "CeraVe", "2027-08-15", "morning", "Daily sun protection"),
    ("Beard Oil", "hair", "Beardbrand", "2027-10-01", "morning", "2-3 drops, comb through"),
    ("Retinol Night Cream", "skincare", "CeraVe", "2026-12-20", "evening", "Pea-sized amount, nights only"),
    ("Face Cleanser", "skincare", "CeraVe", "2027-05-01", "both", "Morning and evening wash"),
    ("Hair Clay", "hair", "Hanz de Fuko", "2026-11-25", "morning", "Matte finish, small amount"),
];

fn seed(conn: &Connection) -> rusqlite::Result<()> {
    for s in SEED_ITEMS {
        conn.execute(
            "INSERT INTO items (name, category, color, color_hex, brand, price,
                 purchase_date, wears, last_worn, size, fit_notes, occasions,
                 material, created_at)
               VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?, date('now'))",
            params![
                s.name, s.category, s.color, s.hex, s.brand, s.price,
                s.purchase_date, s.wears, s.last_worn, s.size, s.fit_notes,
                s.occasions, s.material,
            ],
        )?;
    }
    for g in SEED_GROOMING {
        conn.execute(
            "INSERT INTO grooming (name, category, brand, expiry_date, routine, notes)
             VALUES (?,?,?,?,?,?)",
            params![g.0, g.1, g.2, g.3, g.4, g.5],
        )?;
    }
    let profile = Profile::default();
    conn.execute(
        "INSERT INTO fit_profile (id, sizes, fit_preference, body_notes, height_cm, weight_kg)
         VALUES (1, ?, ?, ?, ?, ?)",
        params![
            serde_json::to_string(&profile.sizes).unwrap(),
            profile.fit_preference,
            profile.body_notes,
            profile.height_cm,
            profile.weight_kg,
        ],
    )?;
    // Yesterday's outfit — drives the no-repeat rule in the demo.
    conn.execute(
        "INSERT INTO outfit_history (date, item_ids, occasion, temp, condition)
         VALUES (date('now', '-1 day'), ?, 'work', 26, 'sunny')",
        params![serde_json::to_string(&vec![2, 7, 11, 18]).unwrap()],
    )?;
    Ok(())
}

// ---------------------------------------------------------------- item CRUD

fn item_from_row(row: &Row) -> rusqlite::Result<Item> {
    let occasions_csv: String = row.get("occasions")?;
    Ok(Item {
        id: row.get("id")?,
        name: row.get("name")?,
        category: row.get("category")?,
        color: row.get("color")?,
        color_hex: row.get("color_hex")?,
        brand: row.get("brand")?,
        price: row.get("price")?,
        purchase_date: row.get("purchase_date")?,
        wears: row.get("wears")?,
        last_worn: row.get("last_worn")?,
        size: row.get("size")?,
        fit_notes: row.get("fit_notes")?,
        occasions: occasions_csv
            .split(',')
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        material: row.get("material")?,
        created_at: row.get("created_at")?,
    })
}

pub fn list_items(
    conn: &Connection,
    category: Option<&str>,
    color: Option<&str>,
    search: Option<&str>,
) -> rusqlite::Result<Vec<Item>> {
    let mut sql = "SELECT * FROM items WHERE 1=1".to_string();
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(c) = category {
        sql += " AND category = ?";
        params.push(c.to_string().into());
    }
    if let Some(c) = color {
        sql += " AND color = ?";
        params.push(c.to_string().into());
    }
    if let Some(s) = search {
        sql += " AND (name LIKE ? OR brand LIKE ? OR color LIKE ?)";
        let like = format!("%{}%", s);
        params.push(like.clone().into());
        params.push(like.clone().into());
        params.push(like.into());
    }
    sql += " ORDER BY category, name";
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), item_from_row)?;
    rows.collect()
}

pub fn get_item(conn: &Connection, item_id: i64) -> rusqlite::Result<Option<Item>> {
    let mut stmt = conn.prepare("SELECT * FROM items WHERE id = ?")?;
    let mut rows = stmt.query(params![item_id])?;
    match rows.next()? {
        Some(row) => Ok(Some(item_from_row(row)?)),
        None => Ok(None),
    }
}

pub fn add_item(conn: &Connection, data: &Value) -> rusqlite::Result<Item> {
    let occasions = match get(data, "occasions") {
        Some(v) => as_occasions(v).join(","),
        None => "casual".to_string(),
    };
    let last_worn: Option<String> = match get(data, "last_worn") {
        Some(Value::Null) | None => None,
        Some(v) => {
            let s = as_str(v);
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
    };
    conn.execute(
        "INSERT INTO items (name, category, color, color_hex, brand, price,
             purchase_date, wears, last_worn, size, fit_notes, occasions,
             material, created_at)
           VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?, date('now'))",
        params![
            get(data, "name").map(as_str).unwrap_or_default(),
            get(data, "category").map(as_str).unwrap_or_default(),
            get(data, "color").map(as_str).unwrap_or_default(),
            get(data, "color_hex").map(as_str).unwrap_or_else(|| "#888888".to_string()),
            get(data, "brand").map(as_str).unwrap_or_default(),
            get(data, "price").map(as_f64).unwrap_or(0.0),
            get(data, "purchase_date").map(as_str).unwrap_or_default(),
            get(data, "wears").map(as_i64).unwrap_or(0),
            last_worn,
            get(data, "size").map(as_str).unwrap_or_default(),
            get(data, "fit_notes").map(as_str).unwrap_or_default(),
            occasions,
            get(data, "material").map(as_str).unwrap_or_default(),
        ],
    )?;
    let id = conn.last_insert_rowid();
    Ok(get_item(conn, id)?.expect("just-inserted item"))
}

pub fn update_item(
    conn: &Connection,
    item_id: i64,
    data: &Value,
) -> rusqlite::Result<Option<Item>> {
    let fields = [
        "name", "category", "color", "color_hex", "brand", "price", "purchase_date",
        "wears", "last_worn", "size", "fit_notes", "occasions", "material",
    ];
    let mut sets: Vec<String> = Vec::new();
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    let obj = match data.as_object() {
        Some(o) => o,
        None => return get_item(conn, item_id),
    };
    for f in fields {
        if let Some(v) = obj.get(f) {
            sets.push(format!("{} = ?", f));
            let val: rusqlite::types::Value = match f {
                "price" => as_f64(v).into(),
                "wears" => as_i64(v).into(),
                "last_worn" => match v {
                    Value::Null => rusqlite::types::Value::Null,
                    _ => {
                        let s = as_str(v);
                        if s.is_empty() {
                            rusqlite::types::Value::Null
                        } else {
                            s.into()
                        }
                    }
                },
                "occasions" => as_occasions(v).join(",").into(),
                _ => as_str(v).into(),
            };
            params.push(val);
        }
    }
    if sets.is_empty() {
        return get_item(conn, item_id);
    }
    params.push(item_id.into());
    let sql = format!("UPDATE items SET {} WHERE id = ?", sets.join(", "));
    conn.execute(&sql, rusqlite::params_from_iter(params.iter()))?;
    get_item(conn, item_id)
}

pub fn delete_item(conn: &Connection, item_id: i64) -> rusqlite::Result<bool> {
    let n = conn.execute("DELETE FROM items WHERE id = ?", params![item_id])?;
    Ok(n > 0)
}

pub fn log_wear(conn: &Connection, item_id: i64, date: &str) -> rusqlite::Result<Option<Item>> {
    conn.execute(
        "UPDATE items SET wears = wears + 1, last_worn = ? WHERE id = ?",
        params![date, item_id],
    )?;
    get_item(conn, item_id)
}

// ---------------------------------------------------------------- fit profile

pub fn get_profile(conn: &Connection) -> rusqlite::Result<Profile> {
    let mut stmt = conn.prepare("SELECT * FROM fit_profile WHERE id = 1")?;
    let mut rows = stmt.query([])?;
    match rows.next()? {
        Some(row) => {
            let sizes_json: String = row.get("sizes")?;
            let sizes: BTreeMap<String, String> =
                serde_json::from_str(&sizes_json).unwrap_or_default();
            Ok(Profile {
                id: row.get("id")?,
                sizes,
                fit_preference: row.get("fit_preference")?,
                body_notes: row.get("body_notes")?,
                height_cm: row.get("height_cm")?,
                weight_kg: row.get("weight_kg")?,
            })
        }
        None => Ok(Profile::default()),
    }
}

pub fn update_profile(conn: &Connection, data: &Value) -> rusqlite::Result<Profile> {
    let cur = get_profile(conn)?;
    let mut sizes = cur.sizes.clone();
    if let Some(Value::Object(m)) = get(data, "sizes") {
        for (k, v) in m {
            sizes.insert(k.clone(), as_str(v));
        }
    }
    let fit_preference = get(data, "fit_preference")
        .map(as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(cur.fit_preference.clone());
    let body_notes = get(data, "body_notes")
        .map(as_str)
        .unwrap_or(cur.body_notes.clone());
    let height_cm: Option<i64> = match get(data, "height_cm") {
        Some(Value::Null) | None => cur.height_cm,
        Some(v) => Some(as_i64(v)),
    };
    let weight_kg: Option<f64> = match get(data, "weight_kg") {
        Some(Value::Null) | None => cur.weight_kg,
        Some(v) => Some(as_f64(v)),
    };
    conn.execute(
        "INSERT INTO fit_profile (id, sizes, fit_preference, body_notes,
             height_cm, weight_kg) VALUES (1, ?, ?, ?, ?, ?)
           ON CONFLICT(id) DO UPDATE SET sizes=excluded.sizes,
             fit_preference=excluded.fit_preference,
             body_notes=excluded.body_notes,
             height_cm=excluded.height_cm, weight_kg=excluded.weight_kg",
        params![
            serde_json::to_string(&sizes).unwrap(),
            fit_preference,
            body_notes,
            height_cm,
            weight_kg,
        ],
    )?;
    get_profile(conn)
}

// ---------------------------------------------------------------- grooming

fn grooming_from_row(row: &Row) -> rusqlite::Result<GroomingProduct> {
    Ok(GroomingProduct {
        id: row.get("id")?,
        name: row.get("name")?,
        category: row.get("category")?,
        brand: row.get("brand")?,
        expiry_date: row.get("expiry_date")?,
        routine: row.get("routine")?,
        notes: row.get("notes")?,
    })
}

pub fn list_grooming(conn: &Connection) -> rusqlite::Result<Vec<GroomingProduct>> {
    let mut stmt = conn.prepare("SELECT * FROM grooming ORDER BY routine, name")?;
    let rows = stmt.query_map([], grooming_from_row)?;
    rows.collect()
}

pub fn add_grooming(conn: &Connection, data: &Value) -> rusqlite::Result<GroomingProduct> {
    conn.execute(
        "INSERT INTO grooming (name, category, brand, expiry_date, routine, notes)
         VALUES (?,?,?,?,?,?)",
        params![
            get(data, "name").map(as_str).unwrap_or_default(),
            get(data, "category").map(as_str).unwrap_or_else(|| "skincare".to_string()),
            get(data, "brand").map(as_str).unwrap_or_default(),
            get(data, "expiry_date").map(as_str).unwrap_or_default(),
            get(data, "routine").map(as_str).unwrap_or_else(|| "morning".to_string()),
            get(data, "notes").map(as_str).unwrap_or_default(),
        ],
    )?;
    let id = conn.last_insert_rowid();
    let mut stmt = conn.prepare("SELECT * FROM grooming WHERE id = ?")?;
    let mut rows = stmt.query(params![id])?;
    Ok(grooming_from_row(rows.next()?.expect("just-inserted product"))?)
}

pub fn delete_grooming(conn: &Connection, product_id: i64) -> rusqlite::Result<bool> {
    let n = conn.execute("DELETE FROM grooming WHERE id = ?", params![product_id])?;
    Ok(n > 0)
}

pub fn get_checks(conn: &Connection, date: &str) -> rusqlite::Result<HashMap<String, bool>> {
    let mut stmt = conn.prepare("SELECT product_id, done FROM routine_checks WHERE date = ?")?;
    let rows = stmt.query_map(params![date], |r| {
        Ok((r.get::<_, i64>("product_id")?, r.get::<_, i64>("done")?))
    })?;
    let mut out = HashMap::new();
    for row in rows {
        let (pid, done) = row?;
        out.insert(pid.to_string(), done != 0);
    }
    Ok(out)
}

pub fn set_check(conn: &Connection, date: &str, product_id: i64, done: bool) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO routine_checks (date, product_id, done) VALUES (?,?,?)
         ON CONFLICT(date, product_id) DO UPDATE SET done=excluded.done",
        params![date, product_id, if done { 1 } else { 0 }],
    )?;
    Ok(())
}

pub fn checks_in_range(conn: &Connection, start: &str, end: &str) -> rusqlite::Result<Vec<Map<String, Value>>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM routine_checks WHERE date >= ? AND date <= ?",
    )?;
    let rows = stmt.query_map(params![start, end], |r| {
        let mut m = Map::new();
        m.insert("date".to_string(), Value::String(r.get("date")?));
        m.insert("product_id".to_string(), json!(r.get::<_, i64>("product_id")?));
        m.insert("done".to_string(), json!(r.get::<_, i64>("done")? != 0));
        Ok(m)
    })?;
    rows.collect()
}

// ---------------------------------------------------------------- outfit history

pub fn record_outfit(
    conn: &Connection,
    date: &str,
    item_ids: &[i64],
    occasion: &str,
    temp: Option<f64>,
    condition: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO outfit_history (date, item_ids, occasion, temp, condition)
         VALUES (?,?,?,?,?)
         ON CONFLICT(date) DO UPDATE SET item_ids=excluded.item_ids,
           occasion=excluded.occasion, temp=excluded.temp, condition=excluded.condition",
        params![
            date,
            serde_json::to_string(item_ids).unwrap(),
            occasion,
            temp,
            condition,
        ],
    )?;
    Ok(())
}

pub fn get_outfit_on(conn: &Connection, date: &str) -> rusqlite::Result<Option<Value>> {
    let mut stmt = conn.prepare("SELECT * FROM outfit_history WHERE date = ?")?;
    let mut rows = stmt.query(params![date])?;
    match rows.next()? {
        Some(row) => {
            let item_ids_json: String = row.get("item_ids")?;
            let item_ids: Vec<i64> = serde_json::from_str(&item_ids_json).unwrap_or_default();
            let temp: Option<f64> = row.get("temp")?;
            Ok(Some(json!({
                "date": row.get::<_, String>("date")?,
                "item_ids": item_ids,
                "occasion": row.get::<_, String>("occasion")?,
                "temp": temp,
                "condition": row.get::<_, String>("condition")?,
            })))
        }
        None => Ok(None),
    }
}

pub fn recent_outfits(conn: &Connection, days: i64) -> rusqlite::Result<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM outfit_history WHERE date >= date('now', ?) ORDER BY date DESC",
    )?;
    let rows = stmt.query_map(params![format!("-{} days", days)], |row| {
        let item_ids_json: String = row.get("item_ids")?;
        let item_ids: Vec<i64> = serde_json::from_str(&item_ids_json).unwrap_or_default();
        let temp: Option<f64> = row.get("temp")?;
        Ok(json!({
            "date": row.get::<_, String>("date")?,
            "item_ids": item_ids,
            "occasion": row.get::<_, String>("occasion")?,
            "temp": temp,
            "condition": row.get::<_, String>("condition")?,
        }))
    })?;
    rows.collect()
}

// ---------------------------------------------------------------- packings

pub fn save_packing(conn: &Connection, data: &Value, items: &Value) -> rusqlite::Result<Value> {
    let occasions = match get(data, "occasions") {
        Some(v) => as_occasions(v).join(","),
        None => String::new(),
    };
    conn.execute(
        "INSERT INTO packings (destination, days, weather, occasions, items, created_at)
         VALUES (?,?,?,?,?, date('now'))",
        params![
            get(data, "destination").map(as_str).unwrap_or_default(),
            get(data, "days").map(as_i64).unwrap_or(0),
            get(data, "weather").map(as_str).unwrap_or_default(),
            occasions,
            serde_json::to_string(items).unwrap(),
        ],
    )?;
    let id = conn.last_insert_rowid();
    let mut stmt = conn.prepare("SELECT * FROM packings WHERE id = ?")?;
    let mut rows = stmt.query(params![id])?;
    let row = rows.next()?.expect("just-inserted packing");
    packing_from_row(row)
}

fn packing_from_row(row: &Row) -> rusqlite::Result<Value> {
    let items_json: String = row.get("items")?;
    let items: Value = serde_json::from_str(&items_json).unwrap_or(Value::Array(vec![]));
    Ok(json!({
        "id": row.get::<_, i64>("id")?,
        "destination": row.get::<_, String>("destination")?,
        "days": row.get::<_, i64>("days")?,
        "weather": row.get::<_, String>("weather")?,
        "occasions": row.get::<_, String>("occasions")?,
        "items": items,
        "created_at": row.get::<_, String>("created_at")?,
    }))
}

pub fn list_packings(conn: &Connection) -> rusqlite::Result<Vec<Value>> {
    let mut stmt = conn.prepare("SELECT * FROM packings ORDER BY id DESC")?;
    let rows = stmt.query_map([], packing_from_row)?;
    rows.collect()
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn test_conn() -> (Connection, String) {
        // Unique path per test: cargo runs tests in parallel threads.
        static COUNTER: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = format!(
            "/tmp/stylesmart_test_db_{}_{}.db",
            std::process::id(),
            n
        );
        let _ = fs::remove_file(&path);
        let conn = init_db(&path).expect("init test db");
        (conn, path)
    }

    #[test]
    fn seed_counts() {
        let (conn, path) = test_conn();
        let items = list_items(&conn, None, None, None).unwrap();
        assert!(items.len() >= 24, "seeded {} items", items.len());
        let cats: std::collections::HashSet<&str> =
            items.iter().map(|i| i.category.as_str()).collect();
        for c in ["top", "bottom", "footwear", "outerwear", "accessory", "grooming", "fragrance"] {
            assert!(cats.contains(c), "missing category {}", c);
        }
        drop(conn);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn crud_roundtrip() {
        let (conn, path) = test_conn();
        let it = add_item(
            &conn,
            &json!({"name": "Test Jacket", "category": "outerwear",
                    "color": "green", "price": 120, "size": "M"}),
        )
        .unwrap();
        assert_eq!(it.name, "Test Jacket");
        let updated = update_item(&conn, it.id, &json!({"price": 99})).unwrap().unwrap();
        assert_eq!(updated.price, 99.0);
        let worn = log_wear(&conn, it.id, "2026-11-11").unwrap().unwrap();
        assert_eq!(worn.wears, 1);
        assert!(delete_item(&conn, it.id).unwrap());
        assert!(get_item(&conn, it.id).unwrap().is_none());
        drop(conn);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn filters() {
        let (conn, path) = test_conn();
        let tops = list_items(&conn, Some("top"), None, None).unwrap();
        assert!(tops.iter().all(|i| i.category == "top"));
        assert!(tops.len() >= 3);
        let found = list_items(&conn, None, None, Some("Levi")).unwrap();
        assert!(found.iter().any(|i| i.brand.contains("Levi")));
        drop(conn);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn profile_roundtrip() {
        let (conn, path) = test_conn();
        let p = get_profile(&conn).unwrap();
        assert_eq!(p.fit_preference, "slim");
        assert_eq!(p.sizes.get("top").map(|s| s.as_str()), Some("M"));
        let p2 = update_profile(&conn, &json!({"fit_preference": "regular"})).unwrap();
        assert_eq!(p2.fit_preference, "regular");
        assert_eq!(p2.sizes.get("top").map(|s| s.as_str()), Some("M"));
        drop(conn);
        let _ = fs::remove_file(&path);
    }
}
