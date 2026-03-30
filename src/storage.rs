//! Save/load benchmark reports to browser localStorage.

use serde::{Deserialize, Serialize};

const STORAGE_KEY: &str = "vello_bench_reports";

/// A single benchmark result within a report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SavedResult {
    pub(crate) name: String,
    pub(crate) ms_per_frame: f64,
    pub(crate) iterations: usize,
}

/// A saved benchmark report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BenchReport {
    pub(crate) label: String,
    pub(crate) viewport_width: u32,
    pub(crate) viewport_height: u32,
    pub(crate) results: Vec<SavedResult>,
}

/// All saved reports (stored as a JSON array).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ReportStore {
    pub(crate) reports: Vec<BenchReport>,
}

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// Load all saved reports from localStorage.
pub(crate) fn load_reports() -> ReportStore {
    let Some(storage) = local_storage() else {
        return ReportStore::default();
    };
    let Some(json) = storage.get_item(STORAGE_KEY).ok().flatten() else {
        return ReportStore::default();
    };
    serde_json::from_str(&json).unwrap_or_default()
}

/// Save a report (appends to the list).
pub(crate) fn save_report(report: BenchReport) {
    let mut store = load_reports();
    store.reports.push(report);
    if let Some(storage) = local_storage()
        && let Ok(json) = serde_json::to_string(&store)
    {
        let _ = storage.set_item(STORAGE_KEY, &json);
    }
}

/// Delete a report by index.
pub(crate) fn delete_report(idx: usize) {
    let mut store = load_reports();
    if idx < store.reports.len() {
        store.reports.remove(idx);
        if let Some(storage) = local_storage()
            && let Ok(json) = serde_json::to_string(&store)
        {
            let _ = storage.set_item(STORAGE_KEY, &json);
        }
    }
}

// ── UI state persistence ─────────────────────────────────────────────────────

const UI_STATE_KEY: &str = "vello_bench_ui_state";

/// Persisted UI state across page reloads.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct UiState {
    /// `"interactive"` or `"benchmark"`.
    pub(crate) mode: Option<String>,
    /// Selected scene index (interactive mode).
    pub(crate) scene: Option<usize>,
    /// Parameter values keyed by name (interactive mode).
    #[serde(default)]
    pub(crate) params: Vec<(String, f64)>,
    /// Checked benchmark indices (benchmark mode).
    #[serde(default)]
    pub(crate) benches: Vec<usize>,
    /// Benchmark preset scale (benchmark mode).
    pub(crate) bench_preset: Option<u32>,
}

/// Load persisted UI state.
pub(crate) fn load_ui_state() -> UiState {
    let Some(storage) = local_storage() else {
        return UiState::default();
    };
    let Some(json) = storage.get_item(UI_STATE_KEY).ok().flatten() else {
        return UiState::default();
    };
    serde_json::from_str(&json).unwrap_or_default()
}

/// Save UI state to localStorage.
pub(crate) fn save_ui_state(state: &UiState) {
    if let Some(storage) = local_storage()
        && let Ok(json) = serde_json::to_string(state)
    {
        let _ = storage.set_item(UI_STATE_KEY, &json);
    }
}
