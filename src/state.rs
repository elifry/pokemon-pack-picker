//! Persisted app state: piles and settings (JSON file), plus pack list and per-pack files.

use crate::models::{PackListEntry, PackRecord, Pile, Settings};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedState {
    pub piles: Vec<Pile>,
    pub settings: Settings,
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            piles: Vec::new(),
            settings: Settings::default(),
        }
    }
}

pub type SharedState = Arc<RwLock<AppState>>;

pub struct AppState {
    /// Persisted piles and settings; use accessors instead of touching directly.
    pub(crate) data: PersistedState,
    /// Path to state.json.
    pub path: PathBuf,
    /// Directory containing state.json; used for packs.json and packs/<id>.json.
    pub data_dir: PathBuf,
}

impl AppState {
    pub fn new(path: PathBuf) -> Self {
        let data_dir = path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            data: PersistedState::default(),
            path,
            data_dir,
        }
    }

    pub fn piles(&self) -> &[Pile] {
        &self.data.piles
    }

    pub fn settings(&self) -> &Settings {
        &self.data.settings
    }

    pub fn settings_mut(&mut self) -> &mut Settings {
        &mut self.data.settings
    }

    pub fn pile_by_id(&self, id: uuid::Uuid) -> Option<&Pile> {
        self.data.piles.iter().find(|p| p.id == id)
    }

    pub fn pile_by_id_mut(&mut self, id: uuid::Uuid) -> Option<&mut Pile> {
        self.data.piles.iter_mut().find(|p| p.id == id)
    }

    pub fn piles_mut(&mut self) -> &mut Vec<Pile> {
        &mut self.data.piles
    }

    pub fn data(&self) -> &PersistedState {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut PersistedState {
        &mut self.data
    }
}

/// Load state from a JSON file; creates default if file missing or invalid.
pub fn load_state(path: &Path) -> Result<PersistedState, Box<dyn std::error::Error + Send + Sync>> {
    let bytes = std::fs::read(path)?;
    let state: PersistedState = serde_json::from_slice(&bytes)?;
    Ok(state)
}

/// Save state to a JSON file.
pub fn save_state(
    path: &Path,
    state: &PersistedState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(state)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

// --------------- Pack list (packs.json) ---------------

pub fn packs_list_path(data_dir: &Path) -> PathBuf {
    data_dir.join("packs.json")
}

pub fn load_packs_list(
    data_dir: &Path,
) -> Result<Vec<PackListEntry>, Box<dyn std::error::Error + Send + Sync>> {
    let path = packs_list_path(data_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(&path)?;
    let list: Vec<PackListEntry> = serde_json::from_slice(&bytes)?;
    Ok(list)
}

pub fn save_packs_list(
    data_dir: &Path,
    list: &[PackListEntry],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    std::fs::create_dir_all(data_dir)?;
    let path = packs_list_path(data_dir);
    let bytes = serde_json::to_vec_pretty(list)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

// --------------- Per-pack file (packs/<id>.json) ---------------

pub fn pack_file_path(data_dir: &Path, pack_id: Uuid) -> PathBuf {
    data_dir.join("packs").join(format!("{}.json", pack_id))
}

pub fn load_pack_record(
    data_dir: &Path,
    pack_id: Uuid,
) -> Result<PackRecord, Box<dyn std::error::Error + Send + Sync>> {
    let path = pack_file_path(data_dir, pack_id);
    let bytes = std::fs::read(&path)?;
    let record: PackRecord = serde_json::from_slice(&bytes)?;
    Ok(record)
}

pub fn save_pack_record(
    data_dir: &Path,
    record: &PackRecord,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let packs_dir = data_dir.join("packs");
    std::fs::create_dir_all(&packs_dir)?;
    let path = pack_file_path(data_dir, record.id);
    let bytes = serde_json::to_vec_pretty(record)?;
    std::fs::write(path, bytes)?;
    Ok(())
}
