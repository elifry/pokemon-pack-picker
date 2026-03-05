//! Pokemon pack picker: web server and routes.

mod models;
mod odds;
mod pack_gen;
mod selection;
mod state;

use axum::http::{StatusCode, header::LOCATION};
use axum::{
    Form, Router,
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
};
use rand::SeedableRng;
use rand::rngs::StdRng;
use state::{
    AppState, SharedState, load_pack_record, load_packs_list, load_state, save_pack_record,
    save_packs_list, save_state,
};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::services::ServeDir;
use tracing::info;
use uuid::Uuid;

// --------------- Hero sprite sheet (expandable grid) ---------------
// Sprite sheet is divided into a grid of cells. One random cell is shown.
// Each cell has four edges; each edge is either Inner (boundary with another cell) or Outer (edge of sheet).
// Corners have 2 outer edges; middle cells have 0 outer edges; others have 1. Trim amounts depend on edge type.

/// Grid size: cols × rows (e.g. 2×2, 3×3, 3×5). Cell (col, row) with col in 0..cols, row in 0..rows.
const HERO_SPRITE_GRID_COLS: u32 = 2;
const HERO_SPRITE_GRID_ROWS: u32 = 2;

/// Trim at inner edges (boundaries between cells). Percent of full image; removes white gap between packs.
const HERO_SPRITE_TRIM_AT_WIDTH_BOUNDARY: f64 = 2.0;
const HERO_SPRITE_TRIM_AT_HEIGHT_BOUNDARY: f64 = 0.0;

/// Trim at outer edges (left/right/top/bottom of the sprite sheet). Percent of full image; set to 0 to keep sheet edges as-is.
const HERO_SPRITE_TRIM_AT_OUTER_WIDTH: f64 = 6.0;
const HERO_SPRITE_TRIM_AT_OUTER_HEIGHT: f64 = 2.0;

#[derive(Clone, Copy)]
enum SpriteEdgeType {
    Inner,
    Outer,
}

/// Classifies the four edges of a cell (top, right, bottom, left). Outer = edge of the sheet; Inner = boundary with another cell.
fn hero_sprite_edge_types(
    col: u32,
    row: u32,
    cols: u32,
    rows: u32,
) -> (
    SpriteEdgeType,
    SpriteEdgeType,
    SpriteEdgeType,
    SpriteEdgeType,
) {
    let top = if row == 0 {
        SpriteEdgeType::Outer
    } else {
        SpriteEdgeType::Inner
    };
    let right = if col + 1 >= cols {
        SpriteEdgeType::Outer
    } else {
        SpriteEdgeType::Inner
    };
    let bottom = if row + 1 >= rows {
        SpriteEdgeType::Outer
    } else {
        SpriteEdgeType::Inner
    };
    let left = if col == 0 {
        SpriteEdgeType::Outer
    } else {
        SpriteEdgeType::Inner
    };
    (top, right, bottom, left)
}

/// Source rect in percent (0–100): (start_x, start_y, width, height). Uses edge types to apply inner vs outer trim per edge.
fn hero_sprite_cell_rect(col: u32, row: u32, cols: u32, rows: u32) -> (f64, f64, f64, f64) {
    let (top, right, bottom, left) = hero_sprite_edge_types(col, row, cols, rows);
    let trim = |edge: SpriteEdgeType, is_width: bool| -> f64 {
        let (inner, outer) = if is_width {
            (
                HERO_SPRITE_TRIM_AT_WIDTH_BOUNDARY,
                HERO_SPRITE_TRIM_AT_OUTER_WIDTH,
            )
        } else {
            (
                HERO_SPRITE_TRIM_AT_HEIGHT_BOUNDARY,
                HERO_SPRITE_TRIM_AT_OUTER_HEIGHT,
            )
        };
        match edge {
            SpriteEdgeType::Inner => inner,
            SpriteEdgeType::Outer => outer,
        }
    };
    let left_trim = trim(left, true);
    let right_trim = trim(right, true);
    let top_trim = trim(top, false);
    let bottom_trim = trim(bottom, false);

    let cell_w_pct = 100.0 / f64::from(cols);
    let cell_h_pct = 100.0 / f64::from(rows);
    let start_x = f64::from(col) * cell_w_pct + left_trim;
    let start_y = f64::from(row) * cell_h_pct + top_trim;
    let width = cell_w_pct - left_trim - right_trim;
    let height = cell_h_pct - top_trim - bottom_trim;
    (start_x, start_y, width, height)
}

fn default_state_path() -> PathBuf {
    std::env::var_os("PPP_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("./data/state.json"))
}

/// Load state from path; if state file contains legacy `pack_history`, migrate to packs.json + packs/<id>.json and rewrite state without it.
fn load_state_and_migrate_packs(path: &std::path::Path) -> (state::PersistedState, ()) {
    if !path.exists() {
        return (
            load_state(path).unwrap_or_else(|_| {
                info!("No state file at {:?}, using default", path);
                state::PersistedState::default()
            }),
            (),
        );
    }
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Could not read state file: {}", e);
            return (state::PersistedState::default(), ());
        }
    };
    let mut value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Invalid state JSON: {}", e);
            return (
                load_state(path).unwrap_or_else(|_| state::PersistedState::default()),
                (),
            );
        }
    };

    let data_dir = path.parent().unwrap_or(std::path::Path::new("."));
    let mut did_migrate = false;
    if let Some(history) = value.get("pack_history").and_then(|v| v.as_array()) {
        let mut list = load_packs_list(data_dir).unwrap_or_default();
        for pack_val in history {
            if let Some(record) =
                serde_json::from_value::<models::PackRecord>(pack_val.clone()).ok()
            {
                if save_pack_record(data_dir, &record).is_ok() {
                    list.push(models::PackListEntry {
                        id: record.id,
                        created_at: record.created_at.clone(),
                        title: if record.title.is_empty() {
                            None
                        } else {
                            Some(record.title.clone())
                        },
                        notes: if record.notes.is_empty() {
                            None
                        } else {
                            Some(record.notes.clone())
                        },
                        card_summary: card_summary_from_slots(&record.slots),
                    });
                }
            }
        }
        list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        if save_packs_list(data_dir, &list).is_ok() {
            info!(
                "Migrated {} packs from state to packs.json + packs/",
                list.len()
            );
            did_migrate = true;
        }
        value.as_object_mut().and_then(|o| o.remove("pack_history"));
    }

    match serde_json::from_value::<state::PersistedState>(value) {
        Ok(s) => {
            if did_migrate {
                let _ = save_state(path, &s);
            }
            (s, ())
        }
        Err(e) => {
            tracing::warn!("State JSON missing required fields: {}", e);
            (state::PersistedState::default(), ())
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?),
        )
        .init();

    let path = default_state_path();
    let (data, _) = load_state_and_migrate_packs(&path);
    let mut app = AppState::new(path.clone());
    app.data = data;
    let app_state: SharedState = Arc::new(RwLock::new(app));

    let app = Router::new()
        .nest_service("/static", ServeDir::new("static"))
        .route("/", get(index))
        .route("/pack", post(generate_pack))
        .route("/pack/result", get(pack_result_placeholder))
        .route("/piles", get(piles_index).post(create_pile))
        .route("/piles/combine", get(combine_form).post(do_combine))
        .route("/piles/split", get(split_form).post(do_split))
        .route("/piles/:id/edit", get(edit_pile_form).post(update_pile))
        .route("/piles/:id/delete", post(delete_pile))
        .route("/settings", get(settings_form).post(update_settings))
        .route("/packs", get(packs_index))
        .route("/packs/:id", get(pack_detail_form).post(update_pack))
        .route("/packs/:id/delete", post(delete_pack))
        .with_state(app_state);

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 3000));
    info!("Listening on http://{}", addr);
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;
    Ok(())
}

async fn save(state: &SharedState) -> Result<(), std::io::Error> {
    let guard = state.read().await;
    save_state(&guard.path, guard.data())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
}

/// Pokémon TCG–style base layout: header with logo + nav, main content, footer. Links to /static/css/style.css.
fn base_layout(title: &str, content: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{} – Pokémon TCG Pack Picker</title>
<link rel="icon" type="image/svg+xml" href="/static/images/logo.svg">
<link rel="stylesheet" href="/static/css/style.css?v=2">
</head>
<body>
<header class="site-header">
<a href="/" class="site-logo"><img src="/static/images/logo.svg" alt=""> Pokémon TCG Pack Picker</a>
<nav class="site-nav">
<a href="/">Home</a>
<a href="/piles">Piles</a>
<a href="/packs">Packs</a>
<a href="/settings">Settings</a>
</nav>
</header>
<main class="main-wrap">
{}
</main>
<footer class="site-footer">Pokémon TCG Pack Picker – Build booster packs from your collection. Not affiliated with The Pokémon Company.</footer>
</body>
</html>"#,
        title, content
    )
}

// --------------- Home ---------------

const HOME_LATEST_PACKS: usize = 4;

async fn index(State(state): State<SharedState>) -> impl IntoResponse {
    let guard = state.read().await;
    let settings_line = format!(
        "{} · {} cards per pack · Energy in packs: {}",
        guard.settings().pack_type.label(),
        guard.settings().pack_size,
        if guard.settings().add_energy_to_packs {
            "Yes"
        } else {
            "No"
        }
    );
    let list = load_packs_list(&guard.data_dir).unwrap_or_default();
    let latest: Vec<_> = list.iter().take(HOME_LATEST_PACKS).collect();
    let latest_packs_html: String = if latest.is_empty() {
        r#"<p class="home-latest-empty">No packs yet. <a href="/">Open a pack</a> to get started.</p>"#.to_string()
    } else {
        latest
            .iter()
            .map(|e| {
                let title = e.title.as_deref().unwrap_or("(no title)");
                let notes = e.notes.as_deref().unwrap_or("");
                let mut block = format!(r#"<a href="/packs/{}" class="home-latest-pack">"#, e.id);
                block.push_str(&format!(
                    r#"<span class="home-latest-title">{}</span>"#,
                    html_escape(title)
                ));
                if !notes.is_empty() {
                    block.push_str(&format!(
                        r#"<span class="home-latest-notes">{}</span>"#,
                        html_escape(notes)
                    ));
                }
                block.push_str("</a>");
                block
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    // Sprite sheet (474×753). Grid and trim in HERO_SPRITE_* constants. Fixed-size layout so pack always visible.
    const SPRITE_W: u32 = 474;
    const SPRITE_H: u32 = 753;
    let pack_h_to_w_ratio = f64::from(SPRITE_H) / f64::from(SPRITE_W);
    let pack_display_w = 200.0_f64; // larger so pack reaches ~bottom of button
    let pack_display_h = pack_display_w * pack_h_to_w_ratio;
    let cols = HERO_SPRITE_GRID_COLS;
    let rows = HERO_SPRITE_GRID_ROWS;
    let img_w = pack_display_w * f64::from(cols);
    let img_h = pack_display_h * f64::from(rows);

    let col = rand::random::<u32>() % cols;
    let row = rand::random::<u32>() % rows;
    let (start_x_pct, start_y_pct, width_pct, height_pct) =
        hero_sprite_cell_rect(col, row, cols, rows);
    let pack_visible_w = img_w * width_pct / 100.0;
    let pack_visible_h = img_h * height_pct / 100.0;
    let img_left_px = -img_w * start_x_pct / 100.0;
    let img_top_px = -img_h * start_y_pct / 100.0;
    let (wrapper_w, wrapper_h, img_width, img_height) = (
        format!("{:.4}px", pack_visible_w),
        format!("{:.4}px", pack_visible_h),
        format!("{:.4}px", img_w),
        format!("{:.4}px", img_h),
    );
    let content = format!(
        r#"<div class="hero">
<div class="hero-visual hero-booster-sprite" role="img" aria-label="Booster pack" style="width: {}; height: {};"><img src="/static/images/booster-sheet.webp" alt="" style="position: absolute; width: {}; height: {}; left: {:.2}px; top: {:.2}px;"></div>
<div class="hero-content">
<h1 class="page-title">Open a Booster Pack</h1>
<p class="page-subtitle">Build packs from your collection with official-style odds. Fill each slot in order using the A/B halving instructions.</p>
<p>Generate one 5-card pack drawn from your piles. You'll get step-by-step instructions for each card so you can pull them blind in the correct order.</p>
<form action="/pack" method="post"><button type="submit" class="btn-primary">Open a Pack</button></form>
</div>
</div>
<div class="card-panel home-two-col">
<div class="home-latest-packs">
<h2>Latest packs</h2>
<div class="home-latest-list">{}</div>
<p><a href="/packs" class="btn-secondary btn-compact">See more</a></p>
</div>
<div class="home-quick-links">
<h2>Quick links</h2>
<ul class="quick-links">
<li><a href="/piles">Manage piles</a> – Add, edit, combine, or split your card piles</li>
<li><a href="/packs">Packs</a> – View and edit opened packs, notes, and what card was in each slot</li>
<li><a href="/settings">Settings</a> – Pack type, pack size, energy, energy types to exclude</li>
</ul>
<p class="settings-summary">Current: {}.</p>
</div>
</div>"#,
        wrapper_w,
        wrapper_h,
        img_width,
        img_height,
        img_left_px,
        img_top_px,
        latest_packs_html,
        settings_line
    );
    Html(base_layout("Home", &content))
}

// --------------- Generate pack ---------------

async fn generate_pack(State(state): State<SharedState>) -> impl IntoResponse {
    let mut guard = state.write().await;
    let mut rng = StdRng::from_entropy();
    match pack_gen::generate_pack(guard.data_mut(), &mut rng) {
        Ok(result) => {
            let data_dir = guard.data_dir.clone();
            drop(guard);
            if let Err(e) = save(&state).await {
                tracing::error!("Failed to save state: {}", e);
            }
            let pack_id = Uuid::new_v4();
            let created_at = chrono::Utc::now().to_rfc3339();
            let record = models::PackRecord {
                id: pack_id,
                created_at: created_at.clone(),
                title: String::new(),
                notes: String::new(),
                slots: result
                    .slots
                    .iter()
                    .map(|s| models::SavedPackSlot {
                        slot_number: s.slot_number,
                        slot_role: s.slot_role.clone(),
                        pile_name: s.pile_name.clone(),
                        instruction_display: s.instruction.display_string(),
                        card_name: None,
                        card_notes: None,
                        recognized_card_id: None,
                        card_holo: None,
                        card_image_url: None,
                    })
                    .collect(),
                warning: result.warning.clone(),
            };
            if save_pack_record(&data_dir, &record).is_ok() {
                let mut list = load_packs_list(&data_dir).unwrap_or_default();
                list.insert(
                    0,
                    models::PackListEntry {
                        id: pack_id,
                        created_at,
                        title: None,
                        notes: None,
                        card_summary: None,
                    },
                );
                let _ = save_packs_list(&data_dir, &list);
            }
            let body = render_pack_result(&result);
            Html(body).into_response()
        }
        Err(e) => {
            let content = format!(
                r#"<h1 class="page-title">Couldn't open a pack</h1>
<div class="card-panel">
<p>{}</p>
<p><a href="/" class="btn-secondary">Back home</a> <a href="/piles" class="btn-secondary">Manage piles</a></p>
</div>"#,
                html_escape(&e)
            );
            Html(base_layout("Error", &content)).into_response()
        }
    }
}

/// Build a single-line summary of card names from pack slots, joined by middle dot (·).
fn card_summary_from_slots(slots: &[models::SavedPackSlot]) -> Option<String> {
    const SEP: &str = " · ";
    let names: Vec<&str> = slots
        .iter()
        .filter_map(|s| s.card_name.as_deref())
        .filter(|s| !s.is_empty())
        .collect();
    if names.is_empty() {
        None
    } else {
        Some(names.join(SEP))
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Format an ISO 8601 / RFC 3339 timestamp as human-readable, e.g. "Monday, July 15, 3:24 PM".
fn format_pack_date(created_at: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(created_at)
        .map(|dt| {
            let s = dt.format("%A, %B %e, %l:%M %p").to_string();
            s.trim().replace("  ", " ").trim().to_string()
        })
        .unwrap_or_else(|_| created_at.to_string())
}

/// Format stored RFC3339 for use in <input type="datetime-local"> (YYYY-MM-DDTHH:mm).
fn created_at_to_datetime_local_value(created_at: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(created_at)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M").to_string())
        .unwrap_or_else(|_| String::new())
}

/// Parse value from datetime-local input (YYYY-MM-DDTHH:mm) and return RFC3339 (UTC, zero seconds).
fn parse_datetime_local_to_rfc3339(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M")
        .ok()
        .map(|naive| {
            chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(naive, chrono::Utc)
                .to_rfc3339()
        })
}

fn render_pack_result(result: &pack_gen::PackResult) -> String {
    let slot_cards: String = result
        .slots
        .iter()
        .map(|s| {
            format!(
                r#"<div class="slot-card"><span class="slot-badge">Slot {} · {}</span><span class="pile-name">{}</span><span class="instruction">{}</span></div>"#,
                s.slot_number,
                s.slot_role,
                html_escape(&s.pile_name),
                html_escape(&s.instruction.display_string())
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let warning = result
        .warning
        .as_ref()
        .map(|w| {
            format!(
                r#"<div class="alert-warning"><strong>Trainer tip:</strong> {}</div>"#,
                html_escape(w)
            )
        })
        .unwrap_or_default();
    let content = format!(
        r#"<h1 class="page-title">Your Booster Pack</h1>
<p class="page-subtitle">Fill each slot in order. Go to the pile, apply the A/B sequence (A = top half, B = bottom half), then use the final number when you have 10 or fewer cards left.</p>
{}
<div class="slot-cards">{}</div>
<div class="pack-result-actions pack-edit-actions"><a href="/" class="btn-primary">Open another pack</a><a href="/piles" class="btn-secondary">My Piles</a></div>"#,
        warning, slot_cards
    );
    base_layout("Your pack", &content)
}

async fn pack_result_placeholder() -> impl IntoResponse {
    Redirect::to("/")
}

// --------------- Piles ---------------

async fn piles_index(State(state): State<SharedState>) -> impl IntoResponse {
    let guard = state.read().await;
    let piles: String = guard
        .piles()
        .iter()
        .map(|p| {
            let typ = pile_type_label(&p.pile_type);
            format!(
                r#"<tr><td>{}</td><td>{}</td><td>{}</td><td class="cell-actions"><a href="/piles/{}/edit">Edit</a> <form action="/piles/{}/delete" method="post" style="display:inline"><button type="submit" class="btn-danger">Delete</button></form></td></tr>"#,
                html_escape(&p.name),
                typ,
                p.estimated_count,
                p.id,
                p.id
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let content = format!(
        r#"<h1 class="page-title">Piles</h1>
<p class="page-subtitle">Manage your card piles. Combine or split piles when you refill or reorganize.</p>
<p><a href="/piles/combine" class="btn-secondary">Combine two piles</a> &nbsp; <a href="/piles/split" class="btn-secondary">Split a pile</a></p>
<div class="card-panel">
<table class="data-table"><thead><tr><th>Pile name</th><th>Type</th><th>Est. count</th><th>Actions</th></tr></thead><tbody>
{}
</tbody></table>
</div>
<div class="card-panel">
<h2>Add a new pile</h2>
<form method="post" action="/piles">
<div class="form-group"><label>Pile name</label><input type="text" name="name" required placeholder="e.g. Bulk commons"></div>
<div class="form-group"><label>Type</label><select name="pile_type">
<option value="bulk">Bulk</option>
<option value="trainers">Trainers</option>
<option value="energy">Energy</option>
<option value="value">Value (price range → rarity)</option>
</select></div>
<div class="form-group"><label>Energy type (if Energy)</label><input type="text" name="energy_type" placeholder="e.g. Fire, Water"></div>
<div class="form-row">
<div class="form-group"><label>Price min $ (if Value)</label><input type="number" step="0.01" name="price_min" placeholder="0"></div>
<div class="form-group"><label>Price max $ (if Value)</label><input type="number" step="0.01" name="price_max" placeholder=""></div>
</div>
<div class="form-group"><label>Estimated count</label><input type="number" name="estimated_count" value="100" min="1" required></div>
<button type="submit" class="btn-primary">Add pile</button>
</form>
</div>"#,
        piles
    );
    Html(base_layout("Piles", &content))
}

fn pile_type_label(t: &models::PileType) -> String {
    match t {
        models::PileType::Trainers => "Trainers".to_string(),
        models::PileType::Energy { energy_type } => format!("Energy ({})", energy_type),
        models::PileType::Bulk => "Bulk".to_string(),
        models::PileType::Value {
            price_min_usd,
            price_max_usd,
            ..
        } => {
            let a = price_min_usd
                .map(|x| format!("${:.2}", x))
                .unwrap_or_default();
            let b = price_max_usd
                .map(|x| format!("${:.2}", x))
                .unwrap_or_default();
            format!("Value {}–{}", a, b)
        }
    }
}

#[derive(serde::Deserialize)]
struct CreatePileForm {
    name: String,
    pile_type: String,
    energy_type: Option<String>,
    price_min: Option<String>,
    price_max: Option<String>,
    estimated_count: u32,
}

async fn create_pile(
    State(state): State<SharedState>,
    Form(f): Form<CreatePileForm>,
) -> impl IntoResponse {
    let pile_type = match f.pile_type.as_str() {
        "trainers" => models::PileType::Trainers,
        "energy" => models::PileType::Energy {
            energy_type: f.energy_type.unwrap_or_else(|| "Basic".to_string()),
        },
        "value" => {
            let min = f.price_min.and_then(|s| s.parse().ok());
            let max = f.price_max.and_then(|s| s.parse().ok());
            models::PileType::Value {
                price_min_usd: min,
                price_max_usd: max,
                rarity: None,
            }
        }
        _ => models::PileType::Bulk,
    };
    let pile = models::Pile::new(f.name.trim().to_string(), pile_type, f.estimated_count);
    {
        let mut guard = state.write().await;
        guard.piles_mut().push(pile);
    }
    if save(&state).await.is_err() {
        tracing::error!("Failed to save state");
    }
    Redirect::to("/piles")
}

async fn edit_pile_form(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let guard = state.read().await;
    let pile = match guard.pile_by_id(id) {
        Some(p) => p,
        None => return Html("Pile not found".to_string()).into_response(),
    };
    let (energy_type, price_min, price_max) = match &pile.pile_type {
        models::PileType::Energy { energy_type } => {
            (energy_type.clone(), String::new(), String::new())
        }
        models::PileType::Value {
            price_min_usd,
            price_max_usd,
            ..
        } => (
            String::new(),
            price_min_usd.map(|x| x.to_string()).unwrap_or_default(),
            price_max_usd.map(|x| x.to_string()).unwrap_or_default(),
        ),
        _ => (String::new(), String::new(), String::new()),
    };
    let content = format!(
        r#"<h1 class="page-title">Edit pile</h1>
<div class="card-panel">
<form id="pile-edit-form" method="post" action="/piles/{}/edit">
<div class="form-group"><label>Pile name</label><input type="text" name="name" value="{}" required></div>
<div class="form-group"><label>Estimated count</label><input type="number" name="estimated_count" value="{}" min="0"></div>
<div class="form-group"><label>Energy type (if Energy)</label><input type="text" name="energy_type" value="{}"></div>
<div class="form-row"><div class="form-group"><label>Price min $</label><input type="text" name="price_min" value="{}"></div><div class="form-group"><label>Price max $</label><input type="text" name="price_max" value="{}"></div></div>
<button type="submit" class="btn-primary">Back to piles</button>
</form>
</div>
<script>
(function() {{
  var form = document.getElementById('pile-edit-form');
  if (!form) return;
  function save() {{
    var fd = new FormData(form);
    fetch(form.action, {{ method: 'POST', body: fd, headers: {{ 'X-Requested-With': 'XMLHttpRequest' }} }});
  }}
  [].slice.call(form.querySelectorAll('input, textarea')).forEach(function(el) {{
    el.addEventListener('blur', save);
    el.addEventListener('change', save);
  }});
}})();
</script>"#,
        id,
        html_escape(&pile.name),
        pile.estimated_count,
        html_escape(&energy_type),
        html_escape(&price_min),
        html_escape(&price_max)
    );
    Html(base_layout("Edit pile", &content)).into_response()
}

#[derive(serde::Deserialize)]
struct UpdatePileForm {
    name: String,
    estimated_count: u32,
    energy_type: Option<String>,
    price_min: Option<String>,
    price_max: Option<String>,
}

async fn update_pile(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    headers: axum::http::HeaderMap,
    Form(f): Form<UpdatePileForm>,
) -> impl IntoResponse {
    let mut guard = state.write().await;
    let pile = match guard.pile_by_id_mut(id) {
        Some(p) => p,
        None => return (StatusCode::FOUND, [(LOCATION, "/piles".to_string())], ()).into_response(),
    };
    pile.name = f.name.trim().to_string();
    pile.estimated_count = f.estimated_count;
    match &mut pile.pile_type {
        models::PileType::Energy { energy_type } => {
            if let Some(t) = f.energy_type {
                *energy_type = t.trim().to_string();
            }
        }
        models::PileType::Value {
            price_min_usd,
            price_max_usd,
            ..
        } => {
            *price_min_usd = f.price_min.and_then(|s| s.parse().ok());
            *price_max_usd = f.price_max.and_then(|s| s.parse().ok());
        }
        _ => {}
    }
    drop(guard);
    if save(&state).await.is_err() {
        tracing::error!("Failed to save state");
    }
    let is_ajax = headers
        .get("X-Requested-With")
        .and_then(|v| v.to_str().ok())
        == Some("XMLHttpRequest");
    if is_ajax {
        (StatusCode::NO_CONTENT, ()).into_response()
    } else {
        (StatusCode::FOUND, [(LOCATION, "/piles".to_string())], ()).into_response()
    }
}

async fn delete_pile(State(state): State<SharedState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    let mut guard = state.write().await;
    guard.piles_mut().retain(|p| p.id != id);
    drop(guard);
    if save(&state).await.is_err() {
        tracing::error!("Failed to save state");
    }
    Redirect::to("/piles")
}

// --------------- Combine ---------------

async fn combine_form(State(state): State<SharedState>) -> impl IntoResponse {
    let guard = state.read().await;
    let options: String = guard
        .piles()
        .iter()
        .map(|p| {
            format!(
                r#"<option value="{}">{}</option>"#,
                p.id,
                html_escape(&p.name)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let content = format!(
        r#"<h1 class="page-title">Combine two piles</h1>
<p class="page-subtitle">Merge two piles into one. The two original piles will be removed.</p>
<div class="card-panel">
<form method="post" action="/piles/combine">
<div class="form-group"><label>First pile</label><select name="id_a" required>{}</select></div>
<div class="form-group"><label>Second pile</label><select name="id_b" required>{}</select></div>
<div class="form-group"><label>New pile name</label><input type="text" name="new_name" required placeholder="e.g. Bulk combined"></div>
<div class="form-group"><label>Estimated total count (or 0 to use sum)</label><input type="number" name="estimated_count" min="0" value="0" placeholder="sum or your estimate"></div>
<button type="submit" class="btn-primary">Combine</button> <a href="/piles" class="btn-secondary">Back to piles</a>
</form>
</div>"#,
        options, options
    );
    Html(base_layout("Combine piles", &content)).into_response()
}

#[derive(serde::Deserialize)]
struct CombineForm {
    id_a: Uuid,
    id_b: Uuid,
    new_name: String,
    estimated_count: u32,
}

async fn do_combine(
    State(state): State<SharedState>,
    Form(f): Form<CombineForm>,
) -> impl IntoResponse {
    let mut guard = state.write().await;
    let (count_a, count_b) = {
        let a = guard.pile_by_id(f.id_a).map(|p| p.estimated_count);
        let b = guard.pile_by_id(f.id_b).map(|p| p.estimated_count);
        (a.unwrap_or(0), b.unwrap_or(0))
    };
    let new_count = if f.estimated_count > 0 {
        f.estimated_count
    } else {
        count_a + count_b
    };
    let typ = guard
        .pile_by_id(f.id_a)
        .map(|p| p.pile_type.clone())
        .unwrap_or(models::PileType::Bulk);
    guard
        .piles_mut()
        .retain(|p| p.id != f.id_a && p.id != f.id_b);
    guard.piles_mut().push(models::Pile::new(
        f.new_name.trim().to_string(),
        typ,
        new_count,
    ));
    drop(guard);
    if save(&state).await.is_err() {
        tracing::error!("Failed to save state");
    }
    Redirect::to("/piles")
}

// --------------- Split ---------------

async fn split_form(State(state): State<SharedState>) -> impl IntoResponse {
    let guard = state.read().await;
    let options: String = guard
        .piles()
        .iter()
        .filter(|p| p.estimated_count > 0)
        .map(|p| {
            format!(
                r#"<option value="{}">{} (est. {})</option>"#,
                p.id,
                html_escape(&p.name),
                p.estimated_count
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let content = format!(
        r#"<h1 class="page-title">Split a pile</h1>
<p class="page-subtitle">Create a new pile from an existing one. Enter the count for the new pile; the original pile's count will be reduced by that amount.</p>
<div class="card-panel">
<form method="post" action="/piles/split">
<div class="form-group"><label>Source pile</label><select name="source_id" required>{}</select></div>
<div class="form-group"><label>New pile name</label><input type="text" name="new_name" required placeholder="e.g. Bulk part 2"></div>
<div class="form-group"><label>Count to split off (into new pile)</label><input type="number" name="split_count" min="1" required></div>
<button type="submit" class="btn-primary">Split</button> <a href="/piles" class="btn-secondary">Back to piles</a>
</form>
</div>"#,
        options
    );
    Html(base_layout("Split pile", &content)).into_response()
}

#[derive(serde::Deserialize)]
struct SplitForm {
    source_id: Uuid,
    new_name: String,
    split_count: u32,
}

async fn do_split(State(state): State<SharedState>, Form(f): Form<SplitForm>) -> impl IntoResponse {
    let mut guard = state.write().await;
    let source = match guard.pile_by_id_mut(f.source_id) {
        Some(p) => p,
        None => return Redirect::to("/piles"),
    };
    let split_count = f.split_count.min(source.estimated_count);
    if split_count == 0 {
        return Redirect::to("/piles");
    }
    let typ = source.pile_type.clone();
    source.estimated_count = source.estimated_count.saturating_sub(split_count);
    guard.piles_mut().push(models::Pile::new(
        f.new_name.trim().to_string(),
        typ,
        split_count,
    ));
    drop(guard);
    if save(&state).await.is_err() {
        tracing::error!("Failed to save state");
    }
    Redirect::to("/piles")
}

// --------------- Packs ---------------

async fn packs_index(State(state): State<SharedState>) -> impl IntoResponse {
    let guard = state.read().await;
    let list = load_packs_list(&guard.data_dir).unwrap_or_default();
    let rows: String = list
        .iter()
        .map(|e| {
            let title = e.title.as_deref().unwrap_or("");
            let notes = e.notes.as_deref().unwrap_or("");
            let cards = e.card_summary.as_deref().unwrap_or("");
            let mut title_cell = if title.is_empty() {
                String::new()
            } else {
                format!(r#"<div>{}</div>"#, html_escape(title))
            };
            if !notes.is_empty() {
                title_cell.push_str(&format!(
                    r#"<div class="pack-list-notes">{}</div>"#,
                    html_escape(notes)
                ));
            }
            if !cards.is_empty() {
                title_cell.push_str(&format!(
                    r#"<div class="pack-list-cards">{}</div>"#,
                    html_escape(cards)
                ));
            }
            format!(
                r#"<tr><td><a href="/packs/{}">View / edit</a></td><td>{}</td><td>{}</td></tr>"#,
                e.id,
                title_cell,
                html_escape(&format_pack_date(&e.created_at))
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let content = format!(
        r#"<h1 class="page-title">Packs</h1>
<p class="page-subtitle">Opened packs. Open a pack from <a href="/">Home</a>; then you can edit title, notes, and what card was pulled per slot.</p>
<div class="card-panel">
<table class="data-table"><thead><tr><th></th><th>Title</th><th>Opened</th></tr></thead><tbody>
{}
</tbody></table>
</div>
<div class="pack-edit-actions"><a href="/" class="btn-primary">Open a pack</a><a href="/piles" class="btn-secondary">Piles</a></div>"#,
        if rows.is_empty() {
            r#"<tr><td colspan="3">No packs yet. <a href="/">Open a pack</a> to get started.</td></tr>"#.to_string()
        } else {
            rows
        }
    );
    Html(base_layout("Packs", &content))
}

async fn pack_detail_form(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let guard = state.read().await;
    let record = match load_pack_record(&guard.data_dir, id) {
        Ok(r) => r,
        Err(_) => return Html(base_layout("Not found", "<p>Pack not found.</p>")).into_response(),
    };
    let title_esc = html_escape(&record.title);
    let notes_esc = html_escape(&record.notes);
    let warning_block = record
        .warning
        .as_ref()
        .map(|w| {
            format!(
                r#"<div class="alert-warning"><strong>Tip:</strong> {}</div>"#,
                html_escape(w)
            )
        })
        .unwrap_or_default();
    let slot_rows: String = record
        .slots
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let card_name = s.card_name.as_deref().unwrap_or("").to_string();
            let card_notes = s.card_notes.as_deref().unwrap_or("").to_string();
            format!(
                r#"<tr><td>Slot {} · {}</td><td>{}</td><td><input type="text" name="slot_{}_card_name" value="{}" placeholder="Card name"></td><td><input type="text" name="slot_{}_card_notes" value="{}" placeholder="Notes"></td></tr>
<tr><td colspan="2"></td><td colspan="2"><span class="muted">Pile: {} · {}</span></td></tr>"#,
                s.slot_number,
                html_escape(&s.slot_role),
                html_escape(&s.instruction_display),
                i,
                html_escape(&card_name),
                i,
                html_escape(&card_notes),
                html_escape(&s.pile_name),
                html_escape(&s.instruction_display)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let content = format!(
        r#"<h1 class="page-title">Pack</h1>
<p class="page-subtitle">Opened {}. Edit title, notes, and what was pulled per slot.</p>
{}
<div class="card-panel">
<form id="pack-edit-form" method="post" action="/packs/{}">
<div class="form-group"><label>Title</label><input type="text" name="title" value="{}" placeholder="e.g. Friday night pull"></div>
<div class="form-group"><label>Date opened</label><input type="datetime-local" name="created_at" value="{}"></div>
<div class="form-group"><label>Notes (e.g. who pulled this pack)</label><textarea name="notes" rows="2" placeholder="e.g. Who pulled, where, etc.">{}</textarea></div>
<table class="data-table"><thead><tr><th>Slot</th><th>Instruction</th><th>Card name</th><th>Card notes</th></tr></thead><tbody>
{}
</tbody></table>
</form>
<div class="pack-edit-actions">
<button type="submit" form="pack-edit-form" class="btn-primary">Back to packs</button>
<form action="/packs/{}/delete" method="post" class="pack-delete-form" onsubmit="return confirm('Are you sure? This pack will be deleted.');">
<button type="submit" class="btn-danger">Delete pack</button>
</form>
</div>
<script>
(function() {{
  var form = document.getElementById('pack-edit-form');
  if (!form) return;
  function save() {{
    var fd = new FormData(form);
    fetch(form.action, {{ method: 'POST', body: fd, headers: {{ 'X-Requested-With': 'XMLHttpRequest' }} }});
  }}
  [].slice.call(form.querySelectorAll('input, textarea')).forEach(function(el) {{
    el.addEventListener('blur', save);
    el.addEventListener('change', save);
  }});
}})();
</script>
</div>"#,
        html_escape(&format_pack_date(&record.created_at)),
        warning_block,
        id,
        title_esc,
        html_escape(&created_at_to_datetime_local_value(&record.created_at)),
        notes_esc,
        slot_rows,
        id
    );
    Html(base_layout("Edit pack", &content)).into_response()
}

#[derive(serde::Deserialize)]
struct UpdatePackForm {
    title: Option<String>,
    created_at: Option<String>,
    notes: Option<String>,
    #[serde(flatten)]
    slots: std::collections::HashMap<String, String>,
}

async fn update_pack(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    headers: axum::http::HeaderMap,
    Form(f): Form<UpdatePackForm>,
) -> impl IntoResponse {
    let guard = state.read().await;
    let mut record = match load_pack_record(&guard.data_dir, id) {
        Ok(r) => r,
        Err(_) => {
            return (StatusCode::FOUND, [(LOCATION, "/packs".to_string())], ()).into_response();
        }
    };
    record.title = f.title.unwrap_or_default().trim().to_string();
    if let Some(ref s) = f.created_at {
        if let Some(rfc3339) = parse_datetime_local_to_rfc3339(s) {
            record.created_at = rfc3339;
        }
    }
    record.notes = f.notes.unwrap_or_default().trim().to_string();
    for (i, slot) in record.slots.iter_mut().enumerate() {
        let name_key = format!("slot_{}_card_name", i);
        let notes_key = format!("slot_{}_card_notes", i);
        if let Some(v) = f.slots.get(&name_key) {
            let s = v.trim().to_string();
            slot.card_name = if s.is_empty() { None } else { Some(s) };
        }
        if let Some(v) = f.slots.get(&notes_key) {
            let s = v.trim().to_string();
            slot.card_notes = if s.is_empty() { None } else { Some(s) };
        }
    }
    let data_dir = guard.data_dir.clone();
    drop(guard);
    if save_pack_record(&data_dir, &record).is_err() {
        tracing::error!("Failed to save pack record");
    }
    if let Ok(mut list) = load_packs_list(&data_dir) {
        if let Some(entry) = list.iter_mut().find(|e| e.id == id) {
            entry.title = if record.title.is_empty() {
                None
            } else {
                Some(record.title.clone())
            };
            entry.notes = if record.notes.is_empty() {
                None
            } else {
                Some(record.notes.clone())
            };
            entry.card_summary = card_summary_from_slots(&record.slots);
            entry.created_at = record.created_at.clone();
        }
        list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        let _ = save_packs_list(&data_dir, &list);
    }
    let is_ajax = headers
        .get("X-Requested-With")
        .and_then(|v| v.to_str().ok())
        == Some("XMLHttpRequest");
    if is_ajax {
        (StatusCode::NO_CONTENT, ()).into_response()
    } else {
        (StatusCode::FOUND, [(LOCATION, "/packs".to_string())], ()).into_response()
    }
}

async fn delete_pack(State(state): State<SharedState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    let guard = state.read().await;
    let data_dir = guard.data_dir.clone();
    drop(guard);
    if let Ok(mut list) = state::load_packs_list(&data_dir) {
        list.retain(|e| e.id != id);
        let _ = state::save_packs_list(&data_dir, &list);
    }
    let path = state::pack_file_path(&data_dir, id);
    let _ = std::fs::remove_file(&path);
    (StatusCode::FOUND, [(LOCATION, "/packs".to_string())], ()).into_response()
}

// --------------- Settings ---------------

async fn settings_form(State(state): State<SharedState>) -> impl IntoResponse {
    let guard = state.read().await;
    let s = guard.settings();
    let pack_type_options = [
        (
            models::PackTypeId::Modern,
            s.pack_type == models::PackTypeId::Modern,
        ),
        (
            models::PackTypeId::Classic,
            s.pack_type == models::PackTypeId::Classic,
        ),
        (
            models::PackTypeId::Legacy,
            s.pack_type == models::PackTypeId::Legacy,
        ),
    ]
    .iter()
    .map(|(id, sel)| {
        let val = match id {
            models::PackTypeId::Modern => "modern",
            models::PackTypeId::Classic => "classic",
            models::PackTypeId::Legacy => "legacy",
        };
        let disabled = if id.is_implemented() { "" } else { " disabled" };
        let opt = format!(
            r#"<option value="{}" {}{}>{}</option>"#,
            val,
            if *sel { "selected" } else { "" },
            disabled,
            id.label()
        );
        opt
    })
    .collect::<Vec<_>>()
    .join("\n");
    let energy_out = s.energy_types_out.join(", ");
    let content = format!(
        r#"<h1 class="page-title">Settings</h1>
<p class="page-subtitle">Pack format and energy options. Changes apply to the next pack you open.</p>
<div class="card-panel">
<form id="settings-form" method="post" action="/settings">
<div class="form-group"><label>Cards per pack</label><input type="number" name="pack_size" value="{}" min="1" max="20"></div>
<div class="form-group"><label>Pack type</label><select name="pack_type">{}</select></div>
<div class="form-group"><label class="checkbox-label"><input type="checkbox" name="add_energy" value="1" {}> Add Energy card to packs</label></div>
<div class="form-group"><label>Energy types to exclude (comma-separated)</label><input type="text" name="energy_types_out" value="{}" placeholder="e.g. Fire, Water"></div>
<button type="submit" class="btn-primary">Back home</button>
</form>
</div>
<script>
(function() {{
  var form = document.getElementById('settings-form');
  if (!form) return;
  function save() {{
    var fd = new FormData(form);
    fetch(form.action, {{ method: 'POST', body: fd, headers: {{ 'X-Requested-With': 'XMLHttpRequest' }} }});
  }}
  [].slice.call(form.querySelectorAll('input, select')).forEach(function(el) {{
    el.addEventListener('blur', save);
    el.addEventListener('change', save);
  }});
}})();
</script>"#,
        s.pack_size,
        pack_type_options,
        if s.add_energy_to_packs { "checked" } else { "" },
        html_escape(&energy_out)
    );
    Html(base_layout("Settings", &content)).into_response()
}

#[derive(serde::Deserialize)]
struct SettingsForm {
    pack_size: Option<u32>,
    pack_type: Option<String>,
    add_energy: Option<String>,
    energy_types_out: Option<String>,
}

async fn update_settings(
    State(state): State<SharedState>,
    headers: axum::http::HeaderMap,
    Form(f): Form<SettingsForm>,
) -> impl IntoResponse {
    let mut guard = state.write().await;
    let settings = guard.settings_mut();
    if let Some(n) = f.pack_size {
        settings.pack_size = n.clamp(1, 20);
    }
    if let Some(t) = &f.pack_type {
        settings.pack_type = match t.as_str() {
            "classic" => models::PackTypeId::Classic,
            "legacy" => models::PackTypeId::Legacy,
            _ => models::PackTypeId::Modern,
        };
    }
    settings.add_energy_to_packs = f.add_energy.as_deref() == Some("1");
    if let Some(s) = &f.energy_types_out {
        settings.energy_types_out = s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect();
    }
    drop(guard);
    if save(&state).await.is_err() {
        tracing::error!("Failed to save state");
    }
    let is_ajax = headers
        .get("X-Requested-With")
        .and_then(|v| v.to_str().ok())
        == Some("XMLHttpRequest");
    if is_ajax {
        (StatusCode::NO_CONTENT, ()).into_response()
    } else {
        (StatusCode::FOUND, [(LOCATION, "/".to_string())], ()).into_response()
    }
}
