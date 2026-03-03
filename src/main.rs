//! Pokemon pack picker: web server and routes.

mod models;
mod odds;
mod pack_gen;
mod recognition;
mod selection;
mod state;

use axum::{
    extract::{Multipart, Path, State},
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
    Form, Router,
};
use rand::SeedableRng;
use rand::rngs::StdRng;
use state::{AppState, SharedState, load_state, save_state};
use chrono::Utc;
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
        .route("/pack/history", get(pack_history_index))
        .route("/pack/result/:id", get(pack_result_by_id))
        .route("/pack/result/:id/slot/:slot/scan", post(scan_slot_card))
        .route("/pack/result/:id/slot/:slot/card", post(update_slot_card))
        .route("/settings/image-rec/disable", post(disable_image_rec))
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
<a href="/pack/history">Pack history</a>
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
            let pack_id = Uuid::new_v4();
            let created_at = Utc::now().to_rfc3339();
            let slots: Vec<models::SlotHistoryEntry> = result
                .slots
                .iter()
                .map(|s| models::SlotHistoryEntry {
                    slot_number: s.slot_number,
                    slot_role: s.slot_role.clone(),
                    pile_name: s.pile_name.clone(),
                    instruction_display: s.instruction.display_string(),
                    recognized_card_id: None,
                    card_name: None,
                    card_holo: None,
                    card_image_url: None,
                })
                .collect();
            let entry = models::PackHistoryEntry {
                id: pack_id,
                created_at: created_at.clone(),
                slots,
                warning: result.warning.clone(),
            };
            guard.data.pack_history.insert(0, entry);
            drop(guard);
            if let Err(e) = save(&state).await {
                tracing::error!("Failed to save state: {}", e);
            }
            Redirect::to(&format!("/pack/result/{}", pack_id)).into_response()
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

async fn pack_result_by_id(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let guard = state.read().await;
    let entry = guard
        .data
        .pack_history
        .iter()
        .find(|p| p.id == id);
    let Some(entry) = entry else {
        let content = format!(
            r#"<h1 class="page-title">Pack not found</h1><div class="card-panel"><p>This pack may have been cleared. <a href="/pack/history">View pack history</a> or <a href="/">open a new pack</a>.</p></div>"#
        );
        return Html(base_layout("Pack not found", &content)).into_response();
    };
    let image_rec_enabled = guard.data.settings.image_rec_enabled;
    let slot_cards = render_slot_cards_for_pack(entry, image_rec_enabled, id);
    let warning = entry
        .warning
        .as_ref()
        .map(|w| {
            format!(
                r#"<div class="alert-warning"><strong>Trainer tip:</strong> {}</div>"#,
                html_escape(w)
            )
        })
        .unwrap_or_default();
    let script = include_script_for_pack_result(image_rec_enabled);
    let content = format!(
        r#"<h1 class="page-title">Your Booster Pack</h1>
<p class="page-subtitle">Fill each slot in order. Go to the pile, apply the A/B sequence (A = top half, B = bottom half), then use the final number when you have 10 or fewer cards left. <a href="/pack/history">View past packs</a>.</p>
{}
<div class="slot-cards">{}</div>
<div class="card-panel">
<p><a href="/" class="btn-primary">Open another pack</a> &nbsp; <a href="/piles" class="btn-secondary">My piles</a> &nbsp; <a href="/pack/history" class="btn-secondary">Pack history</a></p>
</div>
<div id="recognition-modal" class="recognition-modal" style="display:none" aria-hidden="true">
<div class="recognition-modal-content">
<p id="recognition-modal-message">Card recognition isn't set up. You can disable it in Settings or set up the local recognition service.</p>
<p><a href="/docs/image-recognition-setup.html" target="_blank" rel="noopener">Setup guide</a></p>
<button type="button" id="recognition-modal-disable" class="btn-secondary">Disable card recognition</button>
<button type="button" id="recognition-modal-keep" class="btn-primary">Keep it on</button>
</div>
</div>
<script>{}</script>"#,
        warning, slot_cards, script
    );
    Html(base_layout("Your pack", &content)).into_response()
}

fn render_slot_cards_for_pack(
    entry: &models::PackHistoryEntry,
    image_rec_enabled: bool,
    pack_id: Uuid,
) -> String {
    entry
        .slots
        .iter()
        .map(|s| {
            let has_card = s.recognized_card_id.is_some()
                || s.card_name.is_some()
                || s.card_image_url.is_some();
            let card_block = if has_card {
                let name = s
                    .card_name
                    .as_deref()
                    .unwrap_or("(Unknown card)");
                let img = s.card_image_url.as_deref().unwrap_or("");
                let holo_str = s
                    .card_holo
                    .map(|h| if h { "Holo" } else { "Non-holo" })
                    .unwrap_or("");
                format!(
                    r#"<span class="slot-card-details"><img src="{}" alt="" class="slot-card-thumb" onerror="this.style.display='none'"><span class="slot-card-name">{}</span><span class="slot-card-holo">{}</span>
<form method="post" action="/pack/result/{}/slot/{}/card" class="slot-card-edit-form" style="display:inline">
<input type="text" name="card_name" value="{}" placeholder="Card name">
<label class="checkbox-label"><input type="checkbox" name="card_holo" value="1" {}> Holo</label>
<button type="submit" class="btn-small">Save</button>
<button type="submit" name="regenerate_image" value="1" class="btn-small">Regenerate image</button>
</form></span>"#,
                    html_escape(img),
                    html_escape(name),
                    html_escape(holo_str),
                    pack_id,
                    s.slot_number,
                    html_escape(name),
                    if s.card_holo == Some(true) {
                        "checked"
                    } else {
                        ""
                    }
                )
            } else {
                String::new()
            };
            let camera_btn = if image_rec_enabled {
                format!(
                    r#"<button type="button" class="btn-camera slot-scan-btn" data-pack-id="{}" data-slot="{}" title="Scan card for this slot" aria-label="Scan card">📷</button>"#,
                    pack_id, s.slot_number
                )
            } else {
                String::new()
            };
            format!(
                r#"<div class="slot-card" data-slot="{}"><span class="slot-badge">Slot {} · {}</span><span class="pile-name">{}</span><span class="instruction">{}</span><span class="slot-card-actions">{}</span>{}</div>"#,
                s.slot_number,
                s.slot_number,
                html_escape(&s.slot_role),
                html_escape(&s.pile_name),
                html_escape(&s.instruction_display),
                camera_btn,
                card_block
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn include_script_for_pack_result(image_rec_enabled: bool) -> String {
    if !image_rec_enabled {
        return String::new();
    }
    // Run after DOM is ready so .slot-scan-btn and #recognition-modal exist when we bind events.
    r#"
function initRecognition(){
  var modal = document.getElementById('recognition-modal');
  var msg = document.getElementById('recognition-modal-message');
  var btnDisable = document.getElementById('recognition-modal-disable');
  var btnKeep = document.getElementById('recognition-modal-keep');
  function showModal(message) {
    if (msg) msg.textContent = message || "Card recognition isn't set up.";
    if (modal) { modal.style.display = 'flex'; modal.setAttribute('aria-hidden', 'false'); }
  }
  function hideModal() {
    if (modal) { modal.style.display = 'none'; modal.setAttribute('aria-hidden', 'true'); }
  }
  if (btnDisable) btnDisable.addEventListener('click', function(){
    fetch('/settings/image-rec/disable', { method: 'POST' }).then(function(){ window.location.reload(); });
  });
  if (btnKeep) btnKeep.addEventListener('click', hideModal);
  var btns = document.querySelectorAll('.slot-scan-btn');
  btns.forEach(function(btn){
    btn.addEventListener('click', function(ev){
      ev.preventDefault();
      var packId = btn.getAttribute('data-pack-id');
      var slot = btn.getAttribute('data-slot');
      if (!packId || !slot) return;
      if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {
        showModal("Camera not supported in this browser. You can disable card recognition in Settings.");
        return;
      }
      navigator.mediaDevices.getUserMedia({ video: { facingMode: 'environment' } }).then(function(stream){
        var v = document.createElement('video');
        v.srcObject = stream;
        v.play();
        v.style.cssText = 'position:fixed;top:0;left:0;width:100%;height:100%;object-fit:cover;z-index:9998;background:#000;';
        document.body.appendChild(v);
        var canvas = document.createElement('canvas');
        function stop(){ stream.getTracks().forEach(function(t){ t.stop(); }); v.remove(); }
        function capture(){
          canvas.width = v.videoWidth;
          canvas.height = v.videoHeight;
          var ctx = canvas.getContext('2d');
          ctx.drawImage(v, 0, 0);
          stop();
          canvas.toBlob(function(blob){
            var fd = new FormData();
            fd.append('image', blob, 'card.jpg');
            fetch('/pack/result/' + packId + '/slot/' + slot + '/scan', { method: 'POST', body: fd })
              .then(function(r){
                if (r.status === 503) throw { notConfigured: true };
                if (!r.ok) throw new Error('Scan failed');
                window.location.reload();
              })
              .catch(function(e){
                if (e && e.notConfigured) showModal("Card recognition isn't set up. Disable it or set up the local service (see Setup guide).");
                else alert('Scan failed. Try again or disable card recognition in Settings.');
              });
          }, 'image/jpeg', 0.9);
        }
        v.addEventListener('loadeddata', function(){ setTimeout(capture, 500); }, { once: true });
      }).catch(function(err){
        showModal("Camera access denied or unavailable. You can disable card recognition in Settings.");
      });
    });
  });
}
if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', initRecognition);
else initRecognition();
"#
    .to_string()
}

async fn pack_history_index(State(state): State<SharedState>) -> impl IntoResponse {
    let guard = state.read().await;
    let list: String = guard
        .data
        .pack_history
        .iter()
        .take(50)
        .map(|p| {
            let date = p.created_at.get(..10).unwrap_or(&p.created_at);
            format!(
                r#"<tr><td><a href="/pack/result/{}">Pack — {}</a></td><td>{} slots</td></tr>"#,
                p.id,
                html_escape(date),
                p.slots.len()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let content = format!(
        r#"<h1 class="page-title">Pack history</h1>
<p class="page-subtitle">Click a pack to see slot instructions and any recognized cards.</p>
<div class="card-panel">
<table class="data-table"><thead><tr><th>Pack</th><th>Slots</th></tr></thead><tbody>
{}
</tbody></table>
</div>
<p><a href="/" class="btn-primary">Open a new pack</a> <a href="/piles" class="btn-secondary">My piles</a></p>"#,
        if list.is_empty() {
            r#"<tr><td colspan="2">No packs yet. <a href="/">Open a pack</a> to get started.</td></tr>"#.to_string()
        } else {
            list
        }
    );
    Html(base_layout("Pack history", &content)).into_response()
}

#[derive(serde::Deserialize)]
struct PackSlotPath {
    id: Uuid,
    slot: u32,
}

async fn scan_slot_card(
    State(state): State<SharedState>,
    Path(p): Path<PackSlotPath>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let id = p.id;
    let slot = p.slot;
    let image_rec_enabled = {
        let guard = state.read().await;
        guard.data.settings.image_rec_enabled
    };
    if !image_rec_enabled {
        return (axum::http::StatusCode::BAD_REQUEST, [("content-type", "application/json")], r#"{"error":"disabled"}"#.to_string()).into_response();
    }
    let service_url = {
        let guard = state.read().await;
        guard
            .data
            .settings
            .image_rec_service_url
            .clone()
            .filter(|u| !u.trim().is_empty())
    };
    let Some(url) = service_url else {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            [("content-type", "application/json")],
            r#"{"error":"not_configured"}"#.to_string(),
        )
            .into_response();
    };
    let mut image_bytes = Vec::new();
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name().as_deref() == Some("image") {
            if let Ok(data) = field.bytes().await {
                image_bytes = data.to_vec();
                break;
            }
        }
    }
    if image_bytes.is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            [("content-type", "application/json")],
            r#"{"error":"no_image"}"#.to_string(),
        )
            .into_response();
    }
    let card_id = match recognition::recognize_card(&url, &image_bytes).await {
        Ok(id) => id,
        Err(_) => {
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                [("content-type", "application/json")],
                r#"{"error":"service_unavailable"}"#.to_string(),
            )
                .into_response();
        }
    };
    let details = match recognition::fetch_card_details(&card_id).await {
        Ok(d) => d,
        Err(_) => {
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                [("content-type", "application/json")],
                r#"{"error":"api_failed"}"#.to_string(),
            )
                .into_response();
        }
    };
    let mut guard = state.write().await;
    let entry = guard
        .data
        .pack_history
        .iter_mut()
        .find(|p| p.id == id);
    if let Some(entry) = entry {
        if let Some(slot_entry) = entry.slots.iter_mut().find(|s| s.slot_number == slot) {
            slot_entry.recognized_card_id = Some(details.id.clone());
            slot_entry.card_name = Some(details.name.clone());
            slot_entry.card_image_url = Some(details.image_url.clone());
        }
    }
    drop(guard);
    if save(&state).await.is_err() {
        tracing::error!("Failed to save state after scan");
    }
    let body = serde_json::json!({ "cardId": card_id }).to_string();
    (
        axum::http::StatusCode::OK,
        [("content-type", "application/json")],
        body,
    )
        .into_response()
}

#[derive(serde::Deserialize)]
struct UpdateSlotCardForm {
    card_name: Option<String>,
    card_holo: Option<String>,
    regenerate_image: Option<String>,
}

async fn update_slot_card(
    State(state): State<SharedState>,
    Path(p): Path<PackSlotPath>,
    Form(f): Form<UpdateSlotCardForm>,
) -> impl IntoResponse {
    let id = p.id;
    let slot = p.slot;
    let mut guard = state.write().await;
    let entry = guard
        .data
        .pack_history
        .iter_mut()
        .find(|p| p.id == id);
    let Some(entry) = entry else {
        return Redirect::to("/pack/history").into_response();
    };
    let Some(slot_entry) = entry.slots.iter_mut().find(|s| s.slot_number == slot) else {
        return Redirect::to(&format!("/pack/result/{}", id)).into_response();
    };
    if let Some(name) = &f.card_name {
        slot_entry.card_name = Some(name.trim().to_string());
    }
    slot_entry.card_holo = Some(f.card_holo.as_deref() == Some("1"));
    if f.regenerate_image.as_deref() == Some("1") {
        if let Some(ref card_id) = slot_entry.recognized_card_id {
            if let Ok(details) = recognition::fetch_card_details(card_id).await {
                slot_entry.card_name = Some(details.name);
                slot_entry.card_image_url = Some(details.image_url);
            }
        }
    }
    drop(guard);
    if save(&state).await.is_err() {
        tracing::error!("Failed to save state after card update");
    }
    Redirect::to(&format!("/pack/result/{}", id)).into_response()
}

async fn disable_image_rec(State(state): State<SharedState>) -> impl IntoResponse {
    let mut guard = state.write().await;
    guard.data.settings.image_rec_enabled = false;
    drop(guard);
    if save(&state).await.is_err() {
        tracing::error!("Failed to save state");
    }
    Redirect::to("/settings").into_response()
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
    let image_rec_url = s.image_rec_service_url.as_deref().unwrap_or("");
    let content = format!(
        r#"<h1 class="page-title">Settings</h1>
<p class="page-subtitle">Pack format and energy options. Changes apply to the next pack you open.</p>
<div class="card-panel">
<form method="post" action="/settings">
<div class="form-group"><label>Cards per pack</label><input type="number" name="pack_size" value="{}" min="1" max="20"></div>
<div class="form-group"><label>Pack type</label><select name="pack_type">{}</select></div>
<div class="form-group"><label class="checkbox-label"><input type="checkbox" name="add_energy" value="1" {}> Add Energy card to packs</label></div>
<div class="form-group"><label>Energy types to exclude (comma-separated)</label><input type="text" name="energy_types_out" value="{}" placeholder="e.g. Fire, Water"></div>
<hr>
<h2>Card recognition (optional)</h2>
<p>When enabled, a camera button appears on each slot so you can scan cards and track what you pulled. <a href="/static/docs/image-recognition-setup.html" target="_blank" rel="noopener">How to set up the local recognition service</a>.</p>
<div class="form-group"><label class="checkbox-label"><input type="checkbox" name="image_rec_enabled" value="1" {}> Enable card recognition (camera scan)</label></div>
<div class="form-group"><label>Recognition service URL</label><input type="url" name="image_rec_service_url" value="{}" placeholder="e.g. http://127.0.0.1:5000"></div>
<button type="submit" class="btn-primary">Save settings</button> <a href="/" class="btn-secondary">Back home</a>
</form>
</div>"#,
        s.pack_size,
        pack_type_options,
        if s.add_energy_to_packs { "checked" } else { "" },
        html_escape(&energy_out),
        if s.image_rec_enabled { "checked" } else { "" },
        html_escape(image_rec_url)
    );
    Html(base_layout("Settings", &content)).into_response()
}

#[derive(serde::Deserialize)]
struct SettingsForm {
    pack_size: Option<u32>,
    pack_type: Option<String>,
    add_energy: Option<String>,
    energy_types_out: Option<String>,
    image_rec_enabled: Option<String>,
    image_rec_service_url: Option<String>,
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
    guard.data.settings.image_rec_enabled = f.image_rec_enabled.as_deref() == Some("1");
    if let Some(s) = &f.image_rec_service_url {
        let u = s.trim().to_string();
        guard.data.settings.image_rec_service_url = if u.is_empty() {
            None
        } else {
            Some(u)
        };
    }
    drop(guard);
    if save(&state).await.is_err() {
        tracing::error!("Failed to save state");
    }
    Redirect::to("/settings")
}
