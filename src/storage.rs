//! Save/load benchmark reports to browser localStorage.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

const STORAGE_KEY: &str = "vello_bench_reports";
const CALIBRATION_KEY: &str = "vello_bench_calibration";

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
const BACKEND_KEY: &str = "vello_bench_renderer";

/// Persisted UI state across page reloads.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct UiState {
    /// `"interactive"` or `"benchmark"`.
    pub(crate) mode: Option<String>,
    /// Whether the interactive sidebar is collapsed.
    pub(crate) sidebar_collapsed: Option<bool>,
    /// Selected scene index (interactive mode).
    pub(crate) scene: Option<usize>,
    /// Parameter values keyed by name (interactive mode).
    #[serde(default)]
    pub(crate) params: Vec<(String, f64)>,
    /// Checked benchmark indices (benchmark mode).
    pub(crate) benches: Option<Vec<usize>>,
    /// Benchmark warmup frames.
    pub(crate) bench_warmup_samples: Option<u32>,
    /// Benchmark measured frames.
    pub(crate) bench_measured_samples: Option<u32>,
    /// Benchmark viewport width.
    pub(crate) bench_viewport_width: Option<u32>,
    /// Benchmark viewport height.
    pub(crate) bench_viewport_height: Option<u32>,
    /// A/B rounds per benchmark pair.
    pub(crate) ab_rounds: Option<u32>,
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

pub(crate) fn load_backend_name() -> Option<String> {
    local_storage()?.get_item(BACKEND_KEY).ok().flatten()
}

pub(crate) fn save_backend_name(name: &str) {
    if let Some(storage) = local_storage() {
        let _ = storage.set_item(BACKEND_KEY, name);
    }
}

// ── Benchmark calibration persistence ───────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct CalibrationStore {
    pub(crate) profiles: Vec<CalibrationProfile>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct CalibrationProfile {
    pub(crate) key: String,
    #[serde(default)]
    pub(crate) counts: HashMap<crate::harness::ScaleGroup, usize>,
}

impl CalibrationProfile {
    pub(crate) fn count_for(&self, group: crate::harness::ScaleGroup) -> Option<usize> {
        self.counts.get(&group).copied()
    }
}

pub(crate) fn calibration_key(backend: &str, simd: bool, width: u32, height: u32) -> String {
    format!(
        "backend={backend};simd={};width={width};height={height}",
        if simd { 1 } else { 0 }
    )
}

pub(crate) fn load_calibration_store() -> CalibrationStore {
    let Some(storage) = local_storage() else {
        return CalibrationStore::default();
    };
    let Some(json) = storage.get_item(CALIBRATION_KEY).ok().flatten() else {
        return CalibrationStore::default();
    };
    serde_json::from_str(&json).unwrap_or_default()
}

pub(crate) fn load_calibration_profile(key: &str) -> Option<CalibrationProfile> {
    load_calibration_store()
        .profiles
        .into_iter()
        .find(|profile| profile.key == key)
}

pub(crate) fn save_calibration_profile(profile: CalibrationProfile) {
    let mut store = load_calibration_store();
    if let Some(existing) = store
        .profiles
        .iter_mut()
        .find(|existing| existing.key == profile.key)
    {
        *existing = profile;
    } else {
        store.profiles.push(profile);
    }
    if let Some(storage) = local_storage()
        && let Ok(json) = serde_json::to_string(&store)
    {
        let _ = storage.set_item(CALIBRATION_KEY, &json);
    }
}
