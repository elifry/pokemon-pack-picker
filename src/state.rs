//! Persisted app state: piles, settings, and pack history (JSON file).

use crate::models::{PackHistoryEntry, Pile, Settings};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedState {
    pub piles: Vec<Pile>,
    pub settings: Settings,
    /// Past packs (newest first) for history view and recognized card display.
    #[serde(default)]
    pub pack_history: Vec<PackHistoryEntry>,
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            piles: Vec::new(),
            settings: Settings::default(),
            pack_history: Vec::new(),
        }
    }
}

pub type SharedState = Arc<RwLock<AppState>>;

pub struct AppState {
    pub data: PersistedState,
    pub path: std::path::PathBuf,
}

impl AppState {
    pub fn new(path: std::path::PathBuf) -> Self {
        Self {
            data: PersistedState::default(),
            path,
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
