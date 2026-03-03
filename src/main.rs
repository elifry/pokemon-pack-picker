//! Pokemon pack picker: web server and routes.

mod models;
mod odds;
mod pack_gen;
mod selection;
mod state;

use axum::{
    Form, Router,
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
};
use rand::SeedableRng;
use rand::rngs::StdRng;
use state::{AppState, SharedState, load_state, save_state};
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
fn hero_sprite_edge_types(col: u32, row: u32, cols: u32, rows: u32) -> (SpriteEdgeType, SpriteEdgeType, SpriteEdgeType, SpriteEdgeType) {
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
            (HERO_SPRITE_TRIM_AT_WIDTH_BOUNDARY, HERO_SPRITE_TRIM_AT_OUTER_WIDTH)
        } else {
            (HERO_SPRITE_TRIM_AT_HEIGHT_BOUNDARY, HERO_SPRITE_TRIM_AT_OUTER_HEIGHT)
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?),
        )
        .init();

    let path = default_state_path();
    let data = load_state(&path).unwrap_or_else(|_| {
        info!("No state file at {:?}, using default", path);
        state::PersistedState::default()
    });
    let app_state: SharedState = Arc::new(RwLock::new(AppState {
        data,
        path: path.clone(),
    }));

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
        .with_state(app_state);

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 3000));
    info!("Listening on http://{}", addr);
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;
    Ok(())
}

async fn save(state: &SharedState) -> Result<(), std::io::Error> {
    let guard = state.read().await;
    save_state(&guard.path, &guard.data)
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
<link rel="stylesheet" href="/static/css/style.css">
</head>
<body>
<header class="site-header">
<a href="/" class="site-logo"><img src="/static/images/logo.svg" alt=""> Pokémon TCG Pack Picker</a>
<nav class="site-nav">
<a href="/">Home</a>
<a href="/piles">My Piles</a>
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

async fn index(State(state): State<SharedState>) -> impl IntoResponse {
    let guard = state.read().await;
    let settings_line = format!(
        "{} · {} cards per pack · Energy in packs: {}",
        guard.data.settings.pack_type.label(),
        guard.data.settings.pack_size,
        if guard.data.settings.add_energy_to_packs {
            "Yes"
        } else {
            "No"
        }
    );
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
<div class="card-panel">
<h2>Quick links</h2>
<ul class="quick-links">
<li><a href="/piles">Manage my piles</a> – Add, edit, combine, or split your card piles</li>
<li><a href="/settings">Settings</a> – Pack type, pack size, energy, energy types to exclude</li>
</ul>
<p class="settings-summary">Current: {}.</p>
</div>"#,
        wrapper_w,
        wrapper_h,
        img_width,
        img_height,
        img_left_px,
        img_top_px,
        settings_line
    );
    Html(base_layout("Home", &content))
}

// --------------- Generate pack ---------------

async fn generate_pack(State(state): State<SharedState>) -> impl IntoResponse {
    let mut guard = state.write().await;
    let mut rng = StdRng::from_entropy();
    match pack_gen::generate_pack(&mut guard.data, &mut rng) {
        Ok(result) => {
            drop(guard);
            if let Err(e) = save(&state).await {
                tracing::error!("Failed to save state: {}", e);
            }
            // Render result directly
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

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
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
<div class="card-panel">
<p><a href="/" class="btn-primary">Open another pack</a> &nbsp; <a href="/piles" class="btn-secondary">My piles</a></p>
</div>"#,
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
        .data
        .piles
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
        r#"<h1 class="page-title">My Piles</h1>
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
    Html(base_layout("My Piles", &content))
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
        guard.data.piles.push(pile);
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
    let pile = match guard.data.piles.iter().find(|p| p.id == id) {
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
<form method="post" action="/piles/{}/edit">
<div class="form-group"><label>Pile name</label><input type="text" name="name" value="{}" required></div>
<div class="form-group"><label>Estimated count</label><input type="number" name="estimated_count" value="{}" min="0"></div>
<div class="form-group"><label>Energy type (if Energy)</label><input type="text" name="energy_type" value="{}"></div>
<div class="form-row"><div class="form-group"><label>Price min $</label><input type="text" name="price_min" value="{}"></div><div class="form-group"><label>Price max $</label><input type="text" name="price_max" value="{}"></div></div>
<button type="submit" class="btn-primary">Save</button> <a href="/piles" class="btn-secondary">Back to piles</a>
</form>
</div>"#,
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
    Form(f): Form<UpdatePileForm>,
) -> impl IntoResponse {
    let mut guard = state.write().await;
    let pile = match guard.data.piles.iter_mut().find(|p| p.id == id) {
        Some(p) => p,
        None => return Redirect::to("/piles"),
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
    Redirect::to("/piles")
}

async fn delete_pile(State(state): State<SharedState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    let mut guard = state.write().await;
    guard.data.piles.retain(|p| p.id != id);
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
        .data
        .piles
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
        let a = guard
            .data
            .piles
            .iter()
            .find(|p| p.id == f.id_a)
            .map(|p| p.estimated_count);
        let b = guard
            .data
            .piles
            .iter()
            .find(|p| p.id == f.id_b)
            .map(|p| p.estimated_count);
        (a.unwrap_or(0), b.unwrap_or(0))
    };
    let new_count = if f.estimated_count > 0 {
        f.estimated_count
    } else {
        count_a + count_b
    };
    let typ = guard
        .data
        .piles
        .iter()
        .find(|p| p.id == f.id_a)
        .map(|p| p.pile_type.clone())
        .unwrap_or(models::PileType::Bulk);
    guard
        .data
        .piles
        .retain(|p| p.id != f.id_a && p.id != f.id_b);
    guard.data.piles.push(models::Pile::new(
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
        .data
        .piles
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
    let source = match guard.data.piles.iter_mut().find(|p| p.id == f.source_id) {
        Some(p) => p,
        None => return Redirect::to("/piles"),
    };
    let split_count = f.split_count.min(source.estimated_count);
    if split_count == 0 {
        return Redirect::to("/piles");
    }
    let typ = source.pile_type.clone();
    source.estimated_count = source.estimated_count.saturating_sub(split_count);
    guard.data.piles.push(models::Pile::new(
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

// --------------- Settings ---------------

async fn settings_form(State(state): State<SharedState>) -> impl IntoResponse {
    let guard = state.read().await;
    let s = &guard.data.settings;
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
        let opt = format!(
            r#"<option value="{}" {}>{}</option>"#,
            val,
            if *sel { "selected" } else { "" },
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
<form method="post" action="/settings">
<div class="form-group"><label>Cards per pack</label><input type="number" name="pack_size" value="{}" min="1" max="20"></div>
<div class="form-group"><label>Pack type</label><select name="pack_type">{}</select></div>
<div class="form-group"><label class="checkbox-label"><input type="checkbox" name="add_energy" value="1" {}> Add Energy card to packs</label></div>
<div class="form-group"><label>Energy types to exclude (comma-separated)</label><input type="text" name="energy_types_out" value="{}" placeholder="e.g. Fire, Water"></div>
<button type="submit" class="btn-primary">Save settings</button> <a href="/" class="btn-secondary">Back home</a>
</form>
</div>"#,
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
    Form(f): Form<SettingsForm>,
) -> impl IntoResponse {
    let mut guard = state.write().await;
    if let Some(n) = f.pack_size {
        guard.data.settings.pack_size = n.clamp(1, 20);
    }
    if let Some(t) = &f.pack_type {
        guard.data.settings.pack_type = match t.as_str() {
            "classic" => models::PackTypeId::Classic,
            "legacy" => models::PackTypeId::Legacy,
            _ => models::PackTypeId::Modern,
        };
    }
    guard.data.settings.add_energy_to_packs = f.add_energy.as_deref() == Some("1");
    if let Some(s) = &f.energy_types_out {
        guard.data.settings.energy_types_out = s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect();
    }
    drop(guard);
    if save(&state).await.is_err() {
        tracing::error!("Failed to save state");
    }
    Redirect::to("/settings")
}
