//! DOM-based UI for Interactive and Benchmark modes.

#![allow(
    clippy::cast_possible_truncation,
    reason = "truncation has no appreciable impact in this benchmark"
)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::harness::{BenchDef, BenchResult};
use crate::scenes::{BenchScene, Param, ParamKind};
use crate::storage::{BenchReport, UiState};
use wasm_bindgen::prelude::*;
use web_sys::{
    Document, Element, HtmlElement, HtmlImageElement, HtmlInputElement, HtmlSelectElement,
};

// ── Helpers ──────────────────────────────────────────────────────────────────

fn doc() -> Document {
    web_sys::window().unwrap().document().unwrap()
}

fn div(d: &Document) -> HtmlElement {
    d.create_element("div").unwrap().dyn_into().unwrap()
}

fn set(el: &HtmlElement, props: &[(&str, &str)]) {
    let s = el.style();
    for &(k, v) in props {
        s.set_property(k, v).unwrap();
    }
}

fn select_style(sel: &HtmlSelectElement) {
    let s = sel.style();
    for &(k, v) in &[
        ("width", "100%"),
        ("padding", "5px 8px"),
        ("background", "#1e1e2e"),
        ("color", "#cdd6f4"),
        ("border", "1px solid #45475a"),
        ("border-radius", "6px"),
        ("font-family", "'JetBrains Mono', monospace"),
        ("font-size", "12px"),
    ] {
        s.set_property(k, v).unwrap();
    }
}

fn format_val(v: f64, step: f64) -> String {
    if step >= 1.0 {
        format!("{}", v as i64)
    } else {
        format!("{v:.1}")
    }
}

// ── Mode ─────────────────────────────────────────────────────────────────────

/// App mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    /// Interactive exploration.
    Interactive,
    /// Automated benchmarks.
    Benchmark,
}

// ── Param control ────────────────────────────────────────────────────────────

enum ParamCtrl {
    Slider(HtmlInputElement),
    Select(HtmlSelectElement),
}

// ── Bench row ────────────────────────────────────────────────────────────────

/// Per-benchmark-row DOM state.
struct BenchRowState {
    checkbox: HtmlInputElement,
    row: HtmlElement,
    status_dot: HtmlElement,
    result_text: HtmlElement,
    result_line: HtmlElement,
    screenshot_data: Rc<RefCell<String>>,
    delta_text: HtmlElement,
    name: &'static str,
}

// ── UI ───────────────────────────────────────────────────────────────────────

/// Full UI state.
pub struct Ui {
    // Layout
    #[allow(dead_code, reason = "kept alive to prevent GC")]
    top_bar: HtmlElement,
    interactive_view: HtmlElement,
    benchmark_view: HtmlElement,

    // Top bar
    tab_interactive: HtmlElement,
    tab_benchmark: HtmlElement,

    // Interactive: sidebar
    sidebar: HtmlElement,
    toggle_btn: HtmlElement,
    sidebar_collapsed: bool,
    fps_label: HtmlElement,
    encode_label: HtmlElement,
    render_label: HtmlElement,
    viewport_label: HtmlElement,
    /// Scene selector.
    pub scene_select: HtmlSelectElement,
    controls: Vec<(ParamCtrl, HtmlElement, &'static str)>,
    /// Reset view button.
    pub reset_view_btn: HtmlElement,

    // Benchmark
    warmup_input: HtmlInputElement,
    run_input: HtmlInputElement,
    /// Start button.
    pub start_btn: HtmlElement,
    /// Per-benchmark-row DOM state (in order of `bench_defs`).
    bench_rows: Vec<BenchRowState>,
    screenshot_img: HtmlImageElement,
    /// Lightbox overlay for full-size screenshot viewing (kept alive for DOM ownership).
    #[allow(dead_code)]
    lightbox: HtmlElement,
    #[allow(dead_code)]
    lightbox_img: HtmlImageElement,

    // Viewport config
    vp_width_input: HtmlInputElement,
    vp_height_input: HtmlInputElement,

    // Save/load
    save_name_input: HtmlInputElement,
    /// Save button.
    pub save_btn: HtmlElement,
    /// Compare dropdown.
    pub compare_select: HtmlSelectElement,
    /// Delete button for saved reports.
    pub delete_btn: HtmlElement,

    /// Currently loaded comparison report (if any).
    compare_report: Option<BenchReport>,

    /// Current mode.
    pub mode: AppMode,

    /// Whether state needs saving to localStorage.
    /// Shared with closures that mark it on param/checkbox changes.
    dirty: Rc<Cell<bool>>,
}

impl std::fmt::Debug for Ui {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ui")
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

impl Ui {
    /// Build the entire UI.
    pub(crate) fn build(
        document: &Document,
        scenes: &[Box<dyn BenchScene>],
        bench_defs: &[BenchDef],
        current_scene: usize,
        vp_w: u32,
        vp_h: u32,
    ) -> Self {
        let body = document.body().unwrap();
        set(
            &body,
            &[
                ("margin", "0"),
                ("padding", "0"),
                ("overflow", "hidden"),
                ("background", "#11111b"),
                ("color", "#cdd6f4"),
                ("font-family", "'JetBrains Mono', ui-monospace, monospace"),
                ("font-size", "13px"),
            ],
        );

        let dirty = Rc::new(Cell::new(false));

        let (top_bar, tab_interactive, tab_benchmark) = build_top_bar(document);
        body.append_child(&top_bar).unwrap();

        let iv = build_interactive_view(document, scenes, current_scene, vp_w, vp_h, &dirty);
        body.append_child(&iv.view).unwrap();

        let benchmark_view = div(document);
        set(
            &benchmark_view,
            &[
                ("position", "fixed"),
                ("top", "40px"),
                ("left", "0"),
                ("right", "0"),
                ("bottom", "0"),
                ("z-index", "5"),
                ("display", "none"),
                ("overflow-y", "auto"),
                ("padding", "16px 16px 16px 12px"),
                ("box-sizing", "border-box"),
            ],
        );

        let bench_layout = div(document);
        set(
            &bench_layout,
            &[
                ("display", "flex"),
                ("gap", "16px"),
                ("align-items", "flex-start"),
            ],
        );

        let cfg = build_bench_config(document, vp_w, vp_h);
        bench_layout.append_child(&cfg.wrapper).unwrap();

        let rows = build_bench_rows(document, bench_defs, &cfg.screenshot_img, &dirty);
        bench_layout.append_child(&rows.container).unwrap();

        benchmark_view.append_child(&bench_layout).unwrap();
        body.append_child(&benchmark_view).unwrap();

        let (lightbox, lightbox_img) = build_lightbox(document, &body, &cfg.screenshot_img);

        let mut ui = Self {
            top_bar,
            interactive_view: iv.view,
            benchmark_view,
            tab_interactive,
            tab_benchmark,
            sidebar: iv.sidebar,
            toggle_btn: iv.toggle_btn,
            sidebar_collapsed: false,
            fps_label: iv.fps_label,
            encode_label: iv.encode_label,
            render_label: iv.render_label,
            viewport_label: iv.viewport_label,
            scene_select: iv.scene_select,
            controls: iv.controls,
            reset_view_btn: iv.reset_view_btn,
            warmup_input: cfg.warmup_input,
            run_input: cfg.run_input,
            start_btn: cfg.start_btn,
            bench_rows: rows.bench_rows,
            screenshot_img: cfg.screenshot_img,
            lightbox,
            lightbox_img,
            vp_width_input: cfg.vp_width_input,
            vp_height_input: cfg.vp_height_input,
            save_name_input: cfg.save_name_input,
            save_btn: cfg.save_btn,
            compare_select: cfg.compare_select,
            delete_btn: cfg.delete_btn,
            compare_report: None,
            mode: AppMode::Benchmark,
            dirty,
        };
        ui.set_mode(AppMode::Benchmark);
        ui
    }

    // ── Mode switching ───────────────────────────────────────────────────

    /// Switch mode.
    pub fn set_mode(&mut self, mode: AppMode) {
        self.mode = mode;
        self.dirty.set(true);
        match mode {
            AppMode::Interactive => {
                self.interactive_view
                    .style()
                    .set_property("display", "block")
                    .unwrap();
                self.benchmark_view
                    .style()
                    .set_property("display", "none")
                    .unwrap();
                style_tab(&self.tab_interactive, true);
                style_tab(&self.tab_benchmark, false);
            }
            AppMode::Benchmark => {
                self.interactive_view
                    .style()
                    .set_property("display", "none")
                    .unwrap();
                self.benchmark_view
                    .style()
                    .set_property("display", "block")
                    .unwrap();
                style_tab(&self.tab_interactive, false);
                style_tab(&self.tab_benchmark, true);
            }
        }
    }

    /// Tab elements for event binding.
    pub fn tab_elements(&self) -> (&HtmlElement, &HtmlElement) {
        (&self.tab_interactive, &self.tab_benchmark)
    }

    // ── Sidebar toggle ───────────────────────────────────────────────────

    /// Toggle sidebar.
    pub fn toggle_sidebar(&mut self) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        if self.sidebar_collapsed {
            self.sidebar
                .style()
                .set_property("transform", "translateX(-100%)")
                .unwrap();
            self.toggle_btn.style().set_property("left", "0").unwrap();
            self.toggle_btn.set_inner_html("&#x25B6;");
        } else {
            self.sidebar
                .style()
                .set_property("transform", "translateX(0)")
                .unwrap();
            self.toggle_btn
                .style()
                .set_property("left", "284px")
                .unwrap();
            self.toggle_btn.set_inner_html("&#x25C0;");
        }
    }

    /// Toggle button for event binding.
    pub fn toggle_btn(&self) -> &HtmlElement {
        &self.toggle_btn
    }

    /// Sidebar element (for hit-testing).
    pub fn sidebar(&self) -> &HtmlElement {
        &self.sidebar
    }

    // ── Interactive displays ─────────────────────────────────────────────

    /// Update FPS/render displays.
    pub fn update_timing(&self, fps: f64, frame_time: f64, encode_ms: f64, total_ms: f64) {
        self.fps_label
            .set_text_content(Some(&format!("FPS: {fps:.1}  ({frame_time:.1}ms)")));
        self.encode_label
            .set_text_content(Some(&format!("Encode: {encode_ms:.2}ms")));
        self.render_label
            .set_text_content(Some(&format!("Frame: {total_ms:.2}ms")));
    }

    /// Update viewport display.
    pub fn update_viewport(&self, w: u32, h: u32) {
        self.viewport_label
            .set_text_content(Some(&format!("Viewport: {w} x {h}")));
    }

    /// Read interactive param values.
    pub fn read_params(&self) -> Vec<(&'static str, f64)> {
        self.controls
            .iter()
            .map(|(ctrl, _, name)| {
                let v: f64 = match ctrl {
                    ParamCtrl::Slider(i) => i.value().parse().unwrap_or(0.0),
                    ParamCtrl::Select(s) => s.value().parse().unwrap_or(0.0),
                };
                (*name, v)
            })
            .collect()
    }

    /// Rebuild interactive params.
    pub fn rebuild_params(&mut self, params: &[Param]) {
        for (ctrl, _, _) in self.controls.drain(..) {
            let el: &Element = match &ctrl {
                ParamCtrl::Slider(i) => i,
                ParamCtrl::Select(s) => s,
            };
            if let Some(row) = el.parent_element() {
                row.remove();
            }
        }
        let document = doc();
        // Insert controls before the reset-view button so they don't end up below it.
        self.controls = build_controls(
            &document,
            &self.sidebar,
            params,
            Some(&self.reset_view_btn),
            Some(&self.dirty),
        );
    }

    /// Selected interactive scene index.
    pub fn selected_scene(&self) -> usize {
        self.scene_select.selected_index() as usize
    }

    /// Return references to all bench row checkboxes (for event wiring).
    pub fn bench_checkbox_elements(&self) -> Vec<&HtmlInputElement> {
        self.bench_rows.iter().map(|r| &r.checkbox).collect()
    }

    // ── Benchmark displays ───────────────────────────────────────────────

    /// Read warmup ms from input.
    pub fn warmup_ms(&self) -> f64 {
        self.warmup_input.value().parse().unwrap_or(250.0)
    }

    /// Read run ms from input.
    pub fn run_ms(&self) -> f64 {
        self.run_input.value().parse().unwrap_or(1000.0)
    }

    /// Start button ref.
    pub fn start_btn(&self) -> &HtmlElement {
        &self.start_btn
    }

    /// Return indices of checked benchmarks.
    pub fn selected_bench_indices(&self) -> Vec<usize> {
        self.bench_rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.checkbox.checked())
            .map(|(i, _)| i)
            .collect()
    }

    /// Reset all rows to idle state before a run.
    pub fn bench_started(&self, selected: &[usize]) {
        self.screenshot_img
            .style()
            .set_property("display", "none")
            .unwrap();
        for (i, r) in self.bench_rows.iter().enumerate() {
            r.result_line
                .style()
                .set_property("display", "none")
                .unwrap();
            r.result_text.set_text_content(Some(""));
            if selected.contains(&i) {
                r.status_dot
                    .style()
                    .set_property("background", "#f9e2af")
                    .unwrap();
                r.row
                    .style()
                    .set_property("border-color", "#313244")
                    .unwrap();
                r.row.style().set_property("background", "#1e1e2e").unwrap();
            } else {
                r.status_dot
                    .style()
                    .set_property("background", "#313244")
                    .unwrap();
                r.row.style().set_property("opacity", "0.4").unwrap();
            }
            r.checkbox.set_disabled(true);
        }
        self.start_btn
            .style()
            .set_property("opacity", "0.4")
            .unwrap();
        self.start_btn
            .style()
            .set_property("pointer-events", "none")
            .unwrap();
    }

    /// Mark a bench as currently running — prominent red-tinted card.
    pub fn bench_set_running(&self, idx: usize) {
        let r = &self.bench_rows[idx];
        r.row
            .style()
            .set_property("border-color", "#f38ba8")
            .unwrap();
        r.row
            .style()
            .set_property("background", "rgba(243, 139, 168, 0.15)")
            .unwrap();
        r.status_dot
            .style()
            .set_property("background", "#f38ba8")
            .unwrap();
    }

    /// Mark a bench as complete with result.
    pub(crate) fn bench_set_done(&self, idx: usize, r: &BenchResult) {
        let br = &self.bench_rows[idx];
        br.row
            .style()
            .set_property("border-color", "#313244")
            .unwrap();
        br.row
            .style()
            .set_property("background", "#1e1e2e")
            .unwrap();
        br.status_dot
            .style()
            .set_property("background", "#a6e3a1")
            .unwrap();
        br.result_text.set_text_content(Some(&format!(
            "{:.2} ms/f  ({} iters)",
            r.ms_per_frame, r.iterations
        )));
        br.result_line
            .style()
            .set_property("display", "flex")
            .unwrap();
        self.show_delta_for(idx, r.ms_per_frame);
    }

    /// Show screenshot from data URL and store it for the given bench index.
    pub fn set_screenshot(&self, bench_idx: usize, data_url: &str) {
        self.screenshot_img.set_src(data_url);
        self.screenshot_img
            .style()
            .set_property("display", "block")
            .unwrap();
        if let Some(br) = self.bench_rows.get(bench_idx) {
            *br.screenshot_data.borrow_mut() = data_url.to_string();
        }
    }

    /// All benchmarks done — re-enable UI and show deltas if comparison loaded.
    pub fn bench_all_done(&self) {
        for r in &self.bench_rows {
            r.checkbox.set_disabled(false);
            r.row.style().set_property("opacity", "1").unwrap();
        }
        self.start_btn.style().set_property("opacity", "1").unwrap();
        self.start_btn
            .style()
            .set_property("pointer-events", "auto")
            .unwrap();
        self.show_deltas();
    }

    // ── Save / Load / Compare ─────────────────────────────────────────

    /// Save current benchmark results to localStorage.
    pub(crate) fn save_results(&self) {
        let label = self.save_name_input.value();
        let label = label.trim().to_string();
        if label.is_empty() {
            return;
        }
        // Prevent duplicate names — overwrite existing report with same name.
        let store = crate::storage::load_reports();
        if let Some(idx) = store.reports.iter().position(|r| r.label == label) {
            crate::storage::delete_report(idx);
        }
        let vp_w: u32 = self.vp_width_input.value().parse().unwrap_or(0);
        let vp_h: u32 = self.vp_height_input.value().parse().unwrap_or(0);

        let mut results = Vec::new();
        for br in &self.bench_rows {
            let text = br.result_text.text_content().unwrap_or_default();
            if text.is_empty() {
                continue;
            }
            // Parse "X.XX ms/f  (N iters)"
            if let Some(ms_str) = text.split(" ms/f").next()
                && let Ok(ms) = ms_str.trim().parse::<f64>()
            {
                let iters = text
                    .split('(')
                    .nth(1)
                    .and_then(|s| s.split(' ').next())
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                results.push(crate::storage::SavedResult {
                    name: br.name.to_string(),
                    ms_per_frame: ms,
                    iterations: iters,
                });
            }
        }

        if results.is_empty() {
            return;
        }

        crate::storage::save_report(BenchReport {
            label,
            viewport_width: vp_w,
            viewport_height: vp_h,
            results,
        });

        self.refresh_compare_dropdown();
    }

    /// Refresh the compare dropdown with current saved reports.
    pub fn refresh_compare_dropdown(&self) {
        // Clear existing options.
        self.compare_select.set_inner_html("");
        let d = doc();
        let none_opt = d.create_element("option").unwrap();
        none_opt.set_text_content(Some("(none)"));
        none_opt.set_attribute("value", "").unwrap();
        self.compare_select.append_child(&none_opt).unwrap();

        let saved = crate::storage::load_reports();
        for (i, r) in saved.reports.iter().enumerate() {
            let opt = d.create_element("option").unwrap();
            let lbl = format!("{} ({}x{})", r.label, r.viewport_width, r.viewport_height);
            opt.set_text_content(Some(&lbl));
            opt.set_attribute("value", &i.to_string()).unwrap();
            self.compare_select.append_child(&opt).unwrap();
        }
    }

    /// Load a comparison report by index, or clear if empty.
    pub fn load_comparison(&mut self) {
        let val = self.compare_select.value();
        if val.is_empty() {
            self.compare_report = None;
            self.hide_deltas();
            return;
        }
        let idx: usize = match val.parse() {
            Ok(i) => i,
            Err(_) => {
                self.compare_report = None;
                self.hide_deltas();
                return;
            }
        };
        let store = crate::storage::load_reports();
        if let Some(report) = store.reports.get(idx).cloned() {
            self.compare_report = Some(report);
            self.show_deltas();
        }
    }

    /// Show delta for a single bench row given its current ms/frame.
    fn show_delta_for(&self, idx: usize, cur_ms: f64) {
        let Some(ref report) = self.compare_report else {
            return;
        };
        let br = &self.bench_rows[idx];
        let base = report
            .results
            .iter()
            .find(|r| r.name == br.name)
            .map(|r| r.ms_per_frame);
        format_delta(&br.delta_text, Some(cur_ms), base);
    }

    /// Show delta indicators comparing current results to loaded report.
    fn show_deltas(&self) {
        let Some(ref report) = self.compare_report else {
            return;
        };
        for br in &self.bench_rows {
            let cur_ms = br
                .result_text
                .text_content()
                .unwrap_or_default()
                .split(" ms/f")
                .next()
                .and_then(|s| s.trim().parse::<f64>().ok());
            let base = report
                .results
                .iter()
                .find(|r| r.name == br.name)
                .map(|r| r.ms_per_frame);
            format_delta(&br.delta_text, cur_ms, base);
        }
    }

    /// Hide all delta indicators.
    pub fn hide_deltas(&self) {
        for r in &self.bench_rows {
            r.delta_text
                .style()
                .set_property("display", "none")
                .unwrap();
        }
    }

    /// Read configured viewport width.
    pub fn configured_viewport(&self) -> (u32, u32) {
        let w: u32 = self.vp_width_input.value().parse().unwrap_or(0);
        let h: u32 = self.vp_height_input.value().parse().unwrap_or(0);
        (w, h)
    }

    /// Save button ref.
    pub fn save_btn(&self) -> &HtmlElement {
        &self.save_btn
    }

    /// Compare select ref.
    pub fn compare_select(&self) -> &HtmlSelectElement {
        &self.compare_select
    }

    /// Mark state as needing a save.
    pub fn mark_dirty(&self) {
        self.dirty.set(true);
    }

    /// If dirty, persist current state to localStorage.
    pub fn flush_state(&self) {
        if self.dirty.get() {
            self.dirty.set(false);
            self.save_state();
        }
    }

    /// Return a clone of the dirty flag for use in closures.
    pub fn dirty_flag(&self) -> Rc<Cell<bool>> {
        self.dirty.clone()
    }

    /// Write current UI state to localStorage.
    pub fn save_state(&self) {
        let mode_str = match self.mode {
            AppMode::Interactive => "interactive",
            AppMode::Benchmark => "benchmark",
        };
        let scene = self.scene_select.selected_index() as usize;
        let params: Vec<(String, f64)> = self
            .controls
            .iter()
            .map(|(ctrl, _, name)| {
                let v: f64 = match ctrl {
                    ParamCtrl::Slider(i) => i.value().parse().unwrap_or(0.0),
                    ParamCtrl::Select(s) => s.value().parse().unwrap_or(0.0),
                };
                (name.to_string(), v)
            })
            .collect();
        let benches: Vec<usize> = self
            .bench_rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.checkbox.checked())
            .map(|(i, _)| i)
            .collect();
        crate::storage::save_ui_state(&UiState {
            mode: Some(mode_str.to_string()),
            scene: Some(scene),
            params,
            benches,
        });
    }

    /// Apply saved bench checkbox selection.
    pub(crate) fn apply_saved_benches(&self, saved: &UiState) {
        if saved.benches.is_empty() {
            return;
        }
        let set: std::collections::HashSet<usize> = saved.benches.iter().copied().collect();
        for (i, r) in self.bench_rows.iter().enumerate() {
            r.checkbox.set_checked(set.contains(&i));
        }
    }

    /// Apply saved interactive param values.
    pub(crate) fn apply_saved_params(&self, saved: &UiState) {
        for (ctrl, val_span, name) in &self.controls {
            if let Some((_, v)) = saved.params.iter().find(|(k, _)| k == name) {
                match ctrl {
                    ParamCtrl::Slider(input) => {
                        input.set_value(&v.to_string());
                        let step: f64 = input.step().parse().unwrap_or(1.0);
                        val_span.set_text_content(Some(&format_val(*v, step)));
                    }
                    ParamCtrl::Select(sel) => {
                        sel.set_value(&v.to_string());
                    }
                }
            }
        }
    }

    /// Delete the currently selected comparison report.
    pub fn delete_selected_report(&mut self) {
        let val = self.compare_select.value();
        if val.is_empty() {
            return;
        }
        let idx: usize = match val.parse() {
            Ok(i) => i,
            Err(_) => return,
        };
        crate::storage::delete_report(idx);
        self.compare_report = None;
        self.hide_deltas();
        self.refresh_compare_dropdown();
    }
}

// ── Delta formatting ─────────────────────────────────────────────────────────

/// Format a percentage delta into a DOM element, coloring by magnitude.
fn format_delta(el: &HtmlElement, cur_ms: Option<f64>, base_ms: Option<f64>) {
    let (Some(cur), Some(base)) = (cur_ms, base_ms) else {
        el.style().set_property("display", "none").unwrap();
        return;
    };
    let pct = ((cur - base) / base) * 100.0;
    let (text, color) = if pct.abs() < 5.0 {
        (format!("{pct:+.1}%"), "#6c7086")
    } else if pct < 0.0 {
        (format!("{pct:+.1}%"), "#a6e3a1")
    } else {
        (format!("+{pct:.1}%"), "#f38ba8")
    };
    el.set_text_content(Some(&text));
    el.style().set_property("color", color).unwrap();
    el.style().set_property("display", "block").unwrap();
}

// ── Builder return types ─────────────────────────────────────────────────────

struct InteractiveViewParts {
    view: HtmlElement,
    sidebar: HtmlElement,
    toggle_btn: HtmlElement,
    fps_label: HtmlElement,
    encode_label: HtmlElement,
    render_label: HtmlElement,
    viewport_label: HtmlElement,
    scene_select: HtmlSelectElement,
    controls: Vec<(ParamCtrl, HtmlElement, &'static str)>,
    reset_view_btn: HtmlElement,
}

struct BenchConfigParts {
    wrapper: HtmlElement,
    warmup_input: HtmlInputElement,
    run_input: HtmlInputElement,
    start_btn: HtmlElement,
    vp_width_input: HtmlInputElement,
    vp_height_input: HtmlInputElement,
    save_name_input: HtmlInputElement,
    save_btn: HtmlElement,
    compare_select: HtmlSelectElement,
    delete_btn: HtmlElement,
    screenshot_img: HtmlImageElement,
}

struct BenchRowsParts {
    container: HtmlElement,
    bench_rows: Vec<BenchRowState>,
}

// ── Sub-builders ─────────────────────────────────────────────────────────────

fn build_top_bar(document: &Document) -> (HtmlElement, HtmlElement, HtmlElement) {
    let top_bar = div(document);
    set(
        &top_bar,
        &[
            ("position", "fixed"),
            ("top", "0"),
            ("left", "0"),
            ("right", "0"),
            ("height", "40px"),
            ("background", "rgba(30, 30, 46, 0.95)"),
            ("backdrop-filter", "blur(8px)"),
            ("display", "flex"),
            ("align-items", "center"),
            ("padding", "0 16px"),
            ("z-index", "100"),
            ("border-bottom", "1px solid #313244"),
        ],
    );

    let logo = div(document);
    logo.set_text_content(Some("vello bench"));
    set(
        &logo,
        &[
            ("color", "#89b4fa"),
            ("font-weight", "700"),
            ("margin-right", "24px"),
            ("font-size", "14px"),
        ],
    );
    top_bar.append_child(&logo).unwrap();

    let tab_interactive = div(document);
    tab_interactive.set_text_content(Some("Interactive"));
    style_tab(&tab_interactive, true);

    let tab_benchmark = div(document);
    tab_benchmark.set_text_content(Some("Benchmark"));
    style_tab(&tab_benchmark, false);

    top_bar.append_child(&tab_benchmark).unwrap();
    top_bar.append_child(&tab_interactive).unwrap();

    let has_toggle = js_sys::Reflect::get(&js_sys::global(), &"__vello_toggle_simd".into())
        .ok()
        .map_or(false, |v| v.is_function());
    if has_toggle {
        let spacer = div(document);
        set(&spacer, &[("flex", "1")]);
        top_bar.append_child(&spacer).unwrap();

        let simd_on = js_sys::Reflect::get(&js_sys::global(), &"__vello_simd".into())
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let simd_btn = div(document);
        simd_btn.set_text_content(Some(if simd_on { "SIMD: ON" } else { "SIMD: OFF" }));
        set(
            &simd_btn,
            &[
                ("color", if simd_on { "#a6e3a1" } else { "#f38ba8" }),
                ("font-size", "12px"),
                ("font-weight", "600"),
                ("cursor", "pointer"),
                ("padding", "4px 10px"),
                ("border-radius", "4px"),
                (
                    "border",
                    if simd_on {
                        "1px solid #a6e3a1"
                    } else {
                        "1px solid #f38ba8"
                    },
                ),
                ("user-select", "none"),
            ],
        );
        {
            let cb = wasm_bindgen::closure::Closure::wrap(Box::new(move || {
                if let Ok(f) =
                    js_sys::Reflect::get(&js_sys::global(), &"__vello_toggle_simd".into())
                {
                    if let Some(f) = f.dyn_ref::<js_sys::Function>() {
                        let _ = f.call0(&wasm_bindgen::JsValue::NULL);
                    }
                }
            }) as Box<dyn FnMut()>);
            simd_btn
                .add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
                .unwrap();
            cb.forget();
        }
        top_bar.append_child(&simd_btn).unwrap();

        // Renderer toggle (hybrid / cpu).
        let renderer_name = js_sys::Reflect::get(&js_sys::global(), &"__vello_renderer".into())
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_else(|| "hybrid".to_string());
        let is_hybrid = renderer_name == "hybrid";
        let renderer_btn = div(document);
        renderer_btn.set_text_content(Some(if is_hybrid { "Hybrid" } else { "CPU" }));
        set(
            &renderer_btn,
            &[
                ("color", "#cdd6f4"),
                ("font-size", "12px"),
                ("font-weight", "600"),
                ("cursor", "pointer"),
                ("padding", "4px 10px"),
                ("border-radius", "4px"),
                ("border", "1px solid #585b70"),
                ("margin-left", "8px"),
                ("user-select", "none"),
            ],
        );
        {
            let cb = wasm_bindgen::closure::Closure::wrap(Box::new(move || {
                if let Ok(f) =
                    js_sys::Reflect::get(&js_sys::global(), &"__vello_toggle_renderer".into())
                {
                    if let Some(f) = f.dyn_ref::<js_sys::Function>() {
                        let _ = f.call0(&wasm_bindgen::JsValue::NULL);
                    }
                }
            }) as Box<dyn FnMut()>);
            renderer_btn
                .add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
                .unwrap();
            cb.forget();
        }
        top_bar.append_child(&renderer_btn).unwrap();
    }

    (top_bar, tab_interactive, tab_benchmark)
}

fn build_interactive_view(
    document: &Document,
    scenes: &[Box<dyn BenchScene>],
    current_scene: usize,
    vp_w: u32,
    vp_h: u32,
    dirty: &Rc<Cell<bool>>,
) -> InteractiveViewParts {
    let view = div(document);
    set(
        &view,
        &[
            ("position", "fixed"),
            ("top", "40px"),
            ("left", "0"),
            ("right", "0"),
            ("bottom", "0"),
            ("z-index", "5"),
            ("pointer-events", "none"),
        ],
    );

    let sidebar = div(document);
    set(
        &sidebar,
        &[
            ("position", "absolute"),
            ("top", "0"),
            ("left", "0"),
            ("width", "280px"),
            ("height", "100%"),
            ("background", "rgba(30, 30, 46, 0.88)"),
            ("backdrop-filter", "blur(8px)"),
            ("padding", "16px"),
            ("box-sizing", "border-box"),
            ("overflow-y", "auto"),
            ("z-index", "10"),
            ("pointer-events", "auto"),
            ("transition", "transform 0.2s ease"),
            ("border-right", "1px solid #313244"),
            ("display", "flex"),
            ("flex-direction", "column"),
        ],
    );

    let toggle_btn = div(document);
    set(
        &toggle_btn,
        &[
            ("position", "absolute"),
            ("top", "8px"),
            ("left", "284px"),
            ("width", "24px"),
            ("height", "24px"),
            ("background", "rgba(30, 30, 46, 0.88)"),
            ("color", "#89b4fa"),
            ("border-radius", "0 6px 6px 0"),
            ("cursor", "pointer"),
            ("z-index", "11"),
            ("pointer-events", "auto"),
            ("display", "flex"),
            ("align-items", "center"),
            ("justify-content", "center"),
            ("font-size", "14px"),
            ("user-select", "none"),
            ("transition", "left 0.2s ease"),
            ("border", "1px solid #313244"),
            ("border-left", "none"),
        ],
    );
    toggle_btn.set_inner_html("&#x25C0;");
    view.append_child(&toggle_btn).unwrap();

    let fps_label = div(document);
    fps_label.set_text_content(Some("FPS: --"));
    set(
        &fps_label,
        &[
            ("font-size", "15px"),
            ("font-weight", "700"),
            ("color", "#a6e3a1"),
            ("margin-bottom", "2px"),
        ],
    );
    sidebar.append_child(&fps_label).unwrap();

    let encode_label = div(document);
    encode_label.set_text_content(Some("Encode: --"));
    set(
        &encode_label,
        &[("color", "#9399b2"), ("margin-bottom", "2px")],
    );
    sidebar.append_child(&encode_label).unwrap();

    let render_label = div(document);
    render_label.set_text_content(Some("Frame: --"));
    set(
        &render_label,
        &[("color", "#9399b2"), ("margin-bottom", "2px")],
    );
    sidebar.append_child(&render_label).unwrap();

    let viewport_label = div(document);
    viewport_label.set_text_content(Some(&format!("Viewport: {vp_w} x {vp_h}")));
    set(
        &viewport_label,
        &[("color", "#6c7086"), ("margin-bottom", "14px")],
    );
    sidebar.append_child(&viewport_label).unwrap();

    let lbl = div(document);
    lbl.set_text_content(Some("Scene"));
    set(
        &lbl,
        &[
            ("color", "#9399b2"),
            ("margin-bottom", "4px"),
            ("font-size", "11px"),
            ("text-transform", "uppercase"),
            ("letter-spacing", "1px"),
        ],
    );
    sidebar.append_child(&lbl).unwrap();

    let scene_select: HtmlSelectElement = document
        .create_element("select")
        .unwrap()
        .dyn_into()
        .unwrap();
    select_style(&scene_select);
    set_prop(&scene_select, "margin-bottom", "16px");
    for (i, s) in scenes.iter().enumerate() {
        let opt = document.create_element("option").unwrap();
        opt.set_text_content(Some(s.name()));
        opt.set_attribute("value", &i.to_string()).unwrap();
        scene_select.append_child(&opt).unwrap();
    }
    scene_select.set_selected_index(current_scene as i32);
    sidebar.append_child(&scene_select).unwrap();

    let sep = div(document);
    set(
        &sep,
        &[
            ("border-top", "1px solid #313244"),
            ("margin", "4px 0 12px 0"),
        ],
    );
    sidebar.append_child(&sep).unwrap();

    let controls = build_controls(
        document,
        &sidebar,
        &scenes[current_scene].params(),
        None,
        Some(dirty),
    );

    let reset_view_btn = div(document);
    reset_view_btn.set_text_content(Some("Reset View"));
    set(
        &reset_view_btn,
        &[
            ("margin-top", "auto"),
            ("padding", "6px 12px"),
            ("background", "#313244"),
            ("color", "#cdd6f4"),
            ("border-radius", "6px"),
            ("cursor", "pointer"),
            ("text-align", "center"),
            ("font-size", "12px"),
            ("user-select", "none"),
            ("display", "none"),
        ],
    );
    sidebar.append_child(&reset_view_btn).unwrap();

    view.append_child(&sidebar).unwrap();

    InteractiveViewParts {
        view,
        sidebar,
        toggle_btn,
        fps_label,
        encode_label,
        render_label,
        viewport_label,
        scene_select,
        controls,
        reset_view_btn,
    }
}

fn build_bench_config(document: &Document, vp_w: u32, vp_h: u32) -> BenchConfigParts {
    let wrapper = div(document);
    set(&wrapper, &[("width", "240px"), ("flex-shrink", "0")]);

    let left_col = div(document);
    set(
        &left_col,
        &[
            ("background", "#1e1e2e"),
            ("border", "1px solid #313244"),
            ("border-radius", "12px"),
            ("padding", "16px"),
            ("box-sizing", "border-box"),
        ],
    );

    let section_label = |doc: &Document, text: &str| -> HtmlElement {
        let el = div(doc);
        el.set_text_content(Some(text));
        set(
            &el,
            &[
                ("color", "#9399b2"),
                ("font-size", "10px"),
                ("text-transform", "uppercase"),
                ("letter-spacing", "1.5px"),
                ("margin-bottom", "8px"),
                ("font-weight", "600"),
            ],
        );
        el
    };

    left_col
        .append_child(&section_label(document, "Run Config"))
        .unwrap();

    let warmup_input = num_input(document, "Warmup", "250");
    warmup_input
        .0
        .style()
        .set_property("margin-bottom", "6px")
        .unwrap();
    left_col.append_child(&warmup_input.0).unwrap();
    let run_input = num_input(document, "Run", "1000");
    run_input
        .0
        .style()
        .set_property("margin-bottom", "12px")
        .unwrap();
    left_col.append_child(&run_input.0).unwrap();

    left_col
        .append_child(&section_label(document, "Viewport"))
        .unwrap();

    let vp_row = div(document);
    set(
        &vp_row,
        &[
            ("display", "flex"),
            ("gap", "6px"),
            ("margin-bottom", "16px"),
            ("align-items", "center"),
        ],
    );
    let vp_width_input = sized_num_input(document, &vp_w.to_string(), "70px");
    vp_row.append_child(&vp_width_input).unwrap();
    let x_label = div(document);
    x_label.set_text_content(Some("x"));
    set(&x_label, &[("color", "#6c7086")]);
    vp_row.append_child(&x_label).unwrap();
    let vp_height_input = sized_num_input(document, &vp_h.to_string(), "70px");
    vp_row.append_child(&vp_height_input).unwrap();
    let px_label = div(document);
    px_label.set_text_content(Some("px"));
    set(&px_label, &[("color", "#6c7086"), ("font-size", "11px")]);
    vp_row.append_child(&px_label).unwrap();
    left_col.append_child(&vp_row).unwrap();

    let start_btn = div(document);
    start_btn.set_text_content(Some("Run Selected"));
    set(
        &start_btn,
        &[
            ("padding", "10px 0"),
            ("background", "#89b4fa"),
            ("color", "#1e1e2e"),
            ("border-radius", "8px"),
            ("font-weight", "700"),
            ("cursor", "pointer"),
            ("user-select", "none"),
            ("font-size", "13px"),
            ("text-align", "center"),
            ("transition", "opacity 0.15s"),
            ("margin-bottom", "16px"),
        ],
    );
    left_col.append_child(&start_btn).unwrap();

    let sep = div(document);
    set(
        &sep,
        &[
            ("border-top", "1px solid #313244"),
            ("margin-bottom", "16px"),
        ],
    );
    left_col.append_child(&sep).unwrap();

    left_col
        .append_child(&section_label(document, "Reports"))
        .unwrap();

    let save_name_input = sized_num_input(document, "baseline", "100%");
    save_name_input.set_type("text");
    save_name_input.set_placeholder("Report name");
    save_name_input
        .style()
        .set_property("margin-bottom", "8px")
        .unwrap();
    save_name_input
        .style()
        .set_property("box-sizing", "border-box")
        .unwrap();
    left_col.append_child(&save_name_input).unwrap();

    let save_btn = div(document);
    save_btn.set_text_content(Some("Save"));
    set(
        &save_btn,
        &[
            ("padding", "7px 0"),
            ("background", "#a6e3a1"),
            ("color", "#1e1e2e"),
            ("border-radius", "6px"),
            ("font-weight", "700"),
            ("cursor", "pointer"),
            ("user-select", "none"),
            ("font-size", "12px"),
            ("text-align", "center"),
            ("margin-bottom", "12px"),
        ],
    );
    left_col.append_child(&save_btn).unwrap();

    let compare_label = div(document);
    compare_label.set_text_content(Some("Compare with"));
    set(
        &compare_label,
        &[
            ("color", "#9399b2"),
            ("font-size", "11px"),
            ("margin-bottom", "4px"),
        ],
    );
    left_col.append_child(&compare_label).unwrap();

    let compare_select: HtmlSelectElement = document
        .create_element("select")
        .unwrap()
        .dyn_into()
        .unwrap();
    select_style(&compare_select);
    compare_select
        .style()
        .set_property("margin-bottom", "8px")
        .unwrap();
    {
        let opt = document.create_element("option").unwrap();
        opt.set_text_content(Some("(none)"));
        opt.set_attribute("value", "").unwrap();
        compare_select.append_child(&opt).unwrap();
    }
    let saved = crate::storage::load_reports();
    for (i, r) in saved.reports.iter().enumerate() {
        let opt = document.create_element("option").unwrap();
        let label = format!("{} ({}x{})", r.label, r.viewport_width, r.viewport_height);
        opt.set_text_content(Some(&label));
        opt.set_attribute("value", &i.to_string()).unwrap();
        compare_select.append_child(&opt).unwrap();
    }
    left_col.append_child(&compare_select).unwrap();

    let delete_btn = div(document);
    delete_btn.set_text_content(Some("Delete Selected Report"));
    set(
        &delete_btn,
        &[
            ("padding", "6px 0"),
            ("background", "#45475a"),
            ("color", "#f38ba8"),
            ("border-radius", "6px"),
            ("font-weight", "600"),
            ("cursor", "pointer"),
            ("user-select", "none"),
            ("font-size", "11px"),
            ("text-align", "center"),
        ],
    );
    left_col.append_child(&delete_btn).unwrap();

    wrapper.append_child(&left_col).unwrap();

    let screenshot_img: HtmlImageElement =
        document.create_element("img").unwrap().dyn_into().unwrap();
    set_prop(&screenshot_img, "width", "100%");
    set_prop(&screenshot_img, "border-radius", "8px");
    set_prop(&screenshot_img, "border", "1px solid #313244");
    set_prop(&screenshot_img, "margin-top", "12px");
    set_prop(&screenshot_img, "display", "none");
    set_prop(&screenshot_img, "cursor", "pointer");
    wrapper.append_child(&screenshot_img).unwrap();

    BenchConfigParts {
        wrapper,
        warmup_input: warmup_input.1,
        run_input: run_input.1,
        start_btn,
        vp_width_input,
        vp_height_input,
        save_name_input,
        save_btn,
        compare_select,
        delete_btn,
        screenshot_img,
    }
}

fn build_bench_rows(
    document: &Document,
    bench_defs: &[BenchDef],
    screenshot_img: &HtmlImageElement,
    dirty: &Rc<Cell<bool>>,
) -> BenchRowsParts {
    let container = div(document);
    set(&container, &[("flex", "1"), ("min-width", "0")]);

    // Global "Select All" toggle
    let select_all_row = div(document);
    set(
        &select_all_row,
        &[
            ("display", "flex"),
            ("align-items", "center"),
            ("gap", "6px"),
            ("margin-bottom", "6px"),
            ("padding", "0 2px"),
        ],
    );
    let select_all_cb: HtmlInputElement = document
        .create_element("input")
        .unwrap()
        .dyn_into()
        .unwrap();
    select_all_cb.set_type("checkbox");
    select_all_cb.set_checked(true);
    set_prop(&select_all_cb, "accent-color", "#89b4fa");
    set_prop(&select_all_cb, "width", "16px");
    set_prop(&select_all_cb, "height", "16px");
    set_prop(&select_all_cb, "cursor", "pointer");
    select_all_row.append_child(&select_all_cb).unwrap();
    let select_all_label = div(document);
    select_all_label.set_text_content(Some("Select All"));
    set(
        &select_all_label,
        &[
            ("color", "#cdd6f4"),
            ("font-weight", "600"),
            ("font-size", "13px"),
            ("cursor", "pointer"),
            ("user-select", "none"),
        ],
    );
    select_all_row.append_child(&select_all_label).unwrap();
    container.append_child(&select_all_row).unwrap();

    let cat_grid = div(document);
    set(
        &cat_grid,
        &[
            ("display", "grid"),
            (
                "grid-template-columns",
                "repeat(auto-fill, minmax(340px, 1fr))",
            ),
            ("gap", "10px"),
            ("align-items", "start"),
        ],
    );

    let mut bench_row_states: Vec<Option<BenchRowState>> =
        (0..bench_defs.len()).map(|_| None).collect();
    let screenshot_img_rc = Rc::new(screenshot_img.clone());

    let mut categories: Vec<&'static str> = Vec::new();
    for def in bench_defs {
        if !categories.contains(&def.category) {
            categories.push(def.category);
        }
    }

    let mut group_checkboxes: Vec<(HtmlInputElement, Vec<usize>)> = Vec::new();

    for cat in &categories {
        let header = div(document);
        set(
            &header,
            &[
                ("display", "flex"),
                ("align-items", "center"),
                ("gap", "6px"),
                ("margin-bottom", "4px"),
                ("margin-top", "2px"),
                ("padding", "0 2px"),
            ],
        );
        let group_cb: HtmlInputElement = document
            .create_element("input")
            .unwrap()
            .dyn_into()
            .unwrap();
        group_cb.set_type("checkbox");
        group_cb.set_checked(true);
        set_prop(&group_cb, "accent-color", "#89b4fa");
        set_prop(&group_cb, "width", "14px");
        set_prop(&group_cb, "height", "14px");
        set_prop(&group_cb, "cursor", "pointer");
        header.append_child(&group_cb).unwrap();
        let cat_label = div(document);
        cat_label.set_text_content(Some(cat));
        set(
            &cat_label,
            &[
                ("color", "#9399b2"),
                ("font-size", "11px"),
                ("text-transform", "uppercase"),
                ("letter-spacing", "1.5px"),
                ("font-weight", "600"),
                ("cursor", "pointer"),
                ("user-select", "none"),
            ],
        );
        header.append_child(&cat_label).unwrap();

        let cat_block = div(document);
        set(
            &cat_block,
            &[
                ("display", "flex"),
                ("flex-direction", "column"),
                ("gap", "4px"),
            ],
        );
        cat_block.append_child(&header).unwrap();

        let mut member_indices = Vec::new();

        for (i, def) in bench_defs.iter().enumerate() {
            if def.category != *cat {
                continue;
            }
            member_indices.push(i);
            bench_row_states[i] = Some(build_single_bench_row(
                document,
                def,
                &screenshot_img_rc,
                &cat_block,
            ));
        }

        cat_grid.append_child(&cat_block).unwrap();
        group_checkboxes.push((group_cb, member_indices));
    }

    let bench_rows: Vec<BenchRowState> = bench_row_states
        .into_iter()
        .map(|s| s.expect("all bench row states should be initialized"))
        .collect();

    // Wire up group checkbox toggle events
    let all_bench_cbs: Rc<Vec<HtmlInputElement>> =
        Rc::new(bench_rows.iter().map(|r| r.checkbox.clone()).collect());

    for (group_cb, member_indices) in &group_checkboxes {
        let cbs = all_bench_cbs.clone();
        let indices = member_indices.clone();
        let gcb = group_cb.clone();
        let dirty = dirty.clone();
        let handler = Closure::wrap(Box::new(move || {
            let checked = gcb.checked();
            for &idx in &indices {
                cbs[idx].set_checked(checked);
            }
            dirty.set(true);
        }) as Box<dyn FnMut()>);
        group_cb
            .add_event_listener_with_callback("change", handler.as_ref().unchecked_ref())
            .unwrap();
        handler.forget();
    }

    // Wire up "Select All" toggle
    {
        let cbs = all_bench_cbs.clone();
        let group_cbs: Vec<HtmlInputElement> =
            group_checkboxes.iter().map(|(cb, _)| cb.clone()).collect();
        let sa_cb = select_all_cb.clone();
        let dirty = dirty.clone();
        let handler = Closure::wrap(Box::new(move || {
            let checked = sa_cb.checked();
            for cb in cbs.iter() {
                cb.set_checked(checked);
            }
            for gcb in &group_cbs {
                gcb.set_checked(checked);
            }
            dirty.set(true);
        }) as Box<dyn FnMut()>);
        select_all_cb
            .add_event_listener_with_callback("change", handler.as_ref().unchecked_ref())
            .unwrap();
        handler.forget();
    }

    container.append_child(&cat_grid).unwrap();

    BenchRowsParts {
        container,
        bench_rows,
    }
}

fn build_single_bench_row(
    document: &Document,
    def: &BenchDef,
    screenshot_img_rc: &Rc<HtmlImageElement>,
    cat_block: &HtmlElement,
) -> BenchRowState {
    let row = div(document);
    set(
        &row,
        &[
            ("background", "#1e1e2e"),
            ("border", "1px solid #313244"),
            ("border-radius", "6px"),
            ("padding", "6px 10px"),
            ("display", "flex"),
            ("align-items", "center"),
            ("gap", "8px"),
            ("transition", "border-color 0.3s, background 0.3s"),
        ],
    );

    let cb: HtmlInputElement = document
        .create_element("input")
        .unwrap()
        .dyn_into()
        .unwrap();
    cb.set_type("checkbox");
    cb.set_checked(true);
    set_prop(&cb, "accent-color", "#89b4fa");
    set_prop(&cb, "width", "14px");
    set_prop(&cb, "height", "14px");
    set_prop(&cb, "cursor", "pointer");
    set_prop(&cb, "flex-shrink", "0");
    {
        let stop = Closure::wrap(Box::new(move |e: web_sys::Event| {
            e.stop_propagation();
        }) as Box<dyn FnMut(_)>);
        cb.add_event_listener_with_callback("click", stop.as_ref().unchecked_ref())
            .unwrap();
        stop.forget();
    }
    row.append_child(&cb).unwrap();

    let dot = div(document);
    set(
        &dot,
        &[
            ("width", "6px"),
            ("height", "6px"),
            ("border-radius", "50%"),
            ("background", "#45475a"),
            ("flex-shrink", "0"),
            ("transition", "background 0.3s"),
        ],
    );
    row.append_child(&dot).unwrap();

    let info = div(document);
    set(&info, &[("flex", "1"), ("min-width", "0")]);

    let name_el = div(document);
    name_el.set_text_content(Some(def.name));
    set(
        &name_el,
        &[
            ("font-weight", "600"),
            ("font-size", "12px"),
            ("color", "#cdd6f4"),
        ],
    );
    info.append_child(&name_el).unwrap();

    let result_line = div(document);
    set(
        &result_line,
        &[
            ("display", "none"),
            ("align-items", "center"),
            ("gap", "6px"),
            ("margin-top", "1px"),
        ],
    );

    let result_text = div(document);
    set(
        &result_text,
        &[
            ("color", "#a6e3a1"),
            ("font-size", "11px"),
            ("white-space", "nowrap"),
        ],
    );
    result_line.append_child(&result_text).unwrap();

    let delta_text = div(document);
    set(
        &delta_text,
        &[
            ("font-size", "11px"),
            ("white-space", "nowrap"),
            ("display", "none"),
            ("font-weight", "600"),
        ],
    );
    result_line.append_child(&delta_text).unwrap();

    info.append_child(&result_line).unwrap();
    row.append_child(&info).unwrap();

    // Info button with custom tooltip
    let info_wrapper = div(document);
    set(
        &info_wrapper,
        &[("position", "relative"), ("flex-shrink", "0")],
    );

    let info_btn = div(document);
    info_btn.set_text_content(Some("ⓘ"));
    set(
        &info_btn,
        &[
            ("color", "#585b70"),
            ("font-size", "13px"),
            ("cursor", "help"),
            ("user-select", "none"),
        ],
    );
    info_wrapper.append_child(&info_btn).unwrap();

    let tooltip = div(document);
    let full_desc = format!(
        "The aim of this benchmark is to measure the performance of {}.",
        def.description
    );
    tooltip.set_text_content(Some(&full_desc));
    set(
        &tooltip,
        &[
            ("display", "none"),
            ("position", "absolute"),
            ("right", "0"),
            ("bottom", "100%"),
            ("margin-bottom", "6px"),
            ("background", "#313244"),
            ("color", "#bac2de"),
            ("font-size", "11px"),
            ("padding", "6px 10px"),
            ("border-radius", "6px"),
            ("white-space", "normal"),
            ("width", "200px"),
            ("z-index", "100"),
            ("pointer-events", "none"),
            ("box-shadow", "0 2px 8px rgba(0,0,0,0.4)"),
        ],
    );
    info_wrapper.append_child(&tooltip).unwrap();

    {
        let tt = tooltip.clone();
        let show = Closure::wrap(Box::new(move || {
            tt.style().set_property("display", "block").unwrap();
        }) as Box<dyn FnMut()>);
        info_btn
            .add_event_listener_with_callback("mouseenter", show.as_ref().unchecked_ref())
            .unwrap();
        show.forget();

        let tt = tooltip.clone();
        let hide = Closure::wrap(Box::new(move || {
            tt.style().set_property("display", "none").unwrap();
        }) as Box<dyn FnMut()>);
        info_btn
            .add_event_listener_with_callback("mouseleave", hide.as_ref().unchecked_ref())
            .unwrap();
        hide.forget();
    }

    row.append_child(&info_wrapper).unwrap();

    // Screenshot storage + click handler
    let screenshot_data: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    {
        let sd = screenshot_data.clone();
        let img = screenshot_img_rc.clone();
        set(&row, &[("cursor", "pointer")]);
        let handler = Closure::wrap(Box::new(move || {
            let url = sd.borrow();
            if !url.is_empty() {
                img.set_src(&url);
                img.style().set_property("display", "block").unwrap();
            }
        }) as Box<dyn FnMut()>);
        row.add_event_listener_with_callback("click", handler.as_ref().unchecked_ref())
            .unwrap();
        handler.forget();
    }

    cat_block.append_child(&row).unwrap();

    BenchRowState {
        checkbox: cb,
        row,
        status_dot: dot,
        result_text,
        result_line,
        screenshot_data,
        delta_text,
        name: def.name,
    }
}

fn build_lightbox(
    document: &Document,
    body: &HtmlElement,
    screenshot_img: &HtmlImageElement,
) -> (HtmlElement, HtmlImageElement) {
    let lightbox = div(document);
    let lightbox_img: HtmlImageElement =
        document.create_element("img").unwrap().dyn_into().unwrap();
    set(
        &lightbox,
        &[
            ("position", "fixed"),
            ("top", "0"),
            ("left", "0"),
            ("width", "100vw"),
            ("height", "100vh"),
            ("display", "none"),
            ("align-items", "center"),
            ("justify-content", "center"),
            ("z-index", "9999"),
            ("background", "rgba(0,0,0,0.75)"),
            ("backdrop-filter", "blur(4px)"),
            ("-webkit-backdrop-filter", "blur(4px)"),
            ("cursor", "pointer"),
        ],
    );
    {
        let s = lightbox_img.style();
        s.set_property("max-width", "85vw").unwrap();
        s.set_property("max-height", "85vh").unwrap();
        s.set_property("border-radius", "8px").unwrap();
        s.set_property("box-shadow", "0 8px 32px rgba(0,0,0,0.6)")
            .unwrap();
        s.set_property("cursor", "default").unwrap();
    }
    lightbox.append_child(&lightbox_img).unwrap();
    body.append_child(&lightbox).unwrap();

    {
        let lb = lightbox.clone();
        let lb_img = lightbox_img.clone();
        let cb = Closure::wrap(Box::new(move |e: web_sys::MouseEvent| {
            if let Some(target) = e.target() {
                if let Ok(node) = target.dyn_into::<web_sys::Node>() {
                    if !lb_img.contains(Some(&node)) {
                        lb.style().set_property("display", "none").unwrap();
                    }
                }
            }
        }) as Box<dyn FnMut(_)>);
        lightbox
            .add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    {
        let lb = lightbox.clone();
        let cb = Closure::wrap(Box::new(move |e: web_sys::KeyboardEvent| {
            if e.key() == "Escape" {
                lb.style().set_property("display", "none").unwrap();
            }
        }) as Box<dyn FnMut(_)>);
        web_sys::window()
            .unwrap()
            .add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    {
        let thumb = screenshot_img.clone();
        let lb = lightbox.clone();
        let lb_img = lightbox_img.clone();
        let cb = Closure::wrap(Box::new(move || {
            let src = thumb.src();
            if !src.is_empty() {
                lb_img.set_src(&src);
                lb.style().set_property("display", "flex").unwrap();
            }
        }) as Box<dyn FnMut()>);
        screenshot_img
            .add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    (lightbox, lightbox_img)
}

// ── Tab styling ──────────────────────────────────────────────────────────────

fn style_tab(el: &HtmlElement, active: bool) {
    set(
        el,
        &[
            ("padding", "8px 16px"),
            ("cursor", "pointer"),
            ("user-select", "none"),
            ("font-size", "13px"),
            ("border-radius", "6px 6px 0 0"),
            ("transition", "color 0.15s, border-color 0.15s"),
            ("border-bottom", "2px solid"),
        ],
    );
    if active {
        el.style().set_property("color", "#89b4fa").unwrap();
        el.style().set_property("border-color", "#89b4fa").unwrap();
    } else {
        el.style().set_property("color", "#6c7086").unwrap();
        el.style()
            .set_property("border-color", "transparent")
            .unwrap();
    }
}

// ── Number input helper ──────────────────────────────────────────────────────

fn num_input(document: &Document, label: &str, default: &str) -> (HtmlElement, HtmlInputElement) {
    let wrapper = div(document);
    set(
        &wrapper,
        &[
            ("display", "flex"),
            ("align-items", "center"),
            ("gap", "6px"),
        ],
    );

    let lbl = div(document);
    lbl.set_text_content(Some(label));
    set(&lbl, &[("color", "#9399b2"), ("font-size", "12px")]);
    wrapper.append_child(&lbl).unwrap();

    let input: HtmlInputElement = document
        .create_element("input")
        .unwrap()
        .dyn_into()
        .unwrap();
    input.set_type("number");
    input.set_value(default);
    set_prop(&input, "width", "70px");
    set_prop(&input, "background", "#1e1e2e");
    set_prop(&input, "color", "#cdd6f4");
    set_prop(&input, "border", "1px solid #45475a");
    set_prop(&input, "border-radius", "6px");
    set_prop(&input, "padding", "4px 8px");
    set_prop(&input, "font-family", "inherit");
    set_prop(&input, "font-size", "12px");
    wrapper.append_child(&input).unwrap();

    let ms = div(document);
    ms.set_text_content(Some("ms"));
    set(&ms, &[("color", "#6c7086"), ("font-size", "11px")]);
    wrapper.append_child(&ms).unwrap();

    (wrapper, input)
}

fn sized_num_input(document: &Document, default: &str, width: &str) -> HtmlInputElement {
    let input: HtmlInputElement = document
        .create_element("input")
        .unwrap()
        .dyn_into()
        .unwrap();
    input.set_type("number");
    input.set_value(default);
    set_prop(&input, "width", width);
    set_prop(&input, "background", "#1e1e2e");
    set_prop(&input, "color", "#cdd6f4");
    set_prop(&input, "border", "1px solid #45475a");
    set_prop(&input, "border-radius", "6px");
    set_prop(&input, "padding", "4px 8px");
    set_prop(&input, "font-family", "inherit");
    set_prop(&input, "font-size", "12px");
    input
}

fn set_prop(el: &impl AsRef<HtmlElement>, k: &str, v: &str) {
    el.as_ref().style().set_property(k, v).unwrap();
}

// ── Param controls ───────────────────────────────────────────────────────────

/// Build parameter controls and attach them to `container`.
///
/// If `insert_before` is `Some`, rows are inserted before that element;
/// otherwise they are appended.
fn build_controls(
    document: &Document,
    container: &Element,
    params: &[Param],
    insert_before: Option<&HtmlElement>,
    dirty: Option<&Rc<Cell<bool>>>,
) -> Vec<(ParamCtrl, HtmlElement, &'static str)> {
    let mut out = Vec::new();

    for p in params {
        let row = div(document);
        set(&row, &[("margin-bottom", "12px")]);

        let label = div(document);
        label.set_text_content(Some(p.label));
        set(
            &label,
            &[
                ("color", "#9399b2"),
                ("margin-bottom", "4px"),
                ("font-size", "11px"),
                ("text-transform", "uppercase"),
                ("letter-spacing", "1px"),
            ],
        );
        row.append_child(&label).unwrap();

        let val_span = div(document);
        set(
            &val_span,
            &[
                ("display", "inline"),
                ("margin-left", "8px"),
                ("color", "#cdd6f4"),
            ],
        );

        let ctrl = match &p.kind {
            ParamKind::Slider { min, max, step } => {
                let input: HtmlInputElement = document
                    .create_element("input")
                    .unwrap()
                    .dyn_into()
                    .unwrap();
                input.set_type("range");
                input.set_min(&min.to_string());
                input.set_max(&max.to_string());
                input.set_step(&step.to_string());
                input.set_value(&p.value.to_string());
                set_prop(&input, "width", "160px");
                set_prop(&input, "vertical-align", "middle");
                set_prop(&input, "accent-color", "#89b4fa");
                row.append_child(&input).unwrap();

                val_span.set_text_content(Some(&format_val(p.value, *step)));
                row.append_child(&val_span).unwrap();

                let vc = val_span.clone();
                let ic = input.clone();
                let st = *step;
                let dirty = dirty.cloned();
                let cb = Closure::wrap(Box::new(move || {
                    let v: f64 = ic.value().parse().unwrap_or(0.0);
                    vc.set_text_content(Some(&format_val(v, st)));
                    if let Some(ref d) = dirty {
                        d.set(true);
                    }
                }) as Box<dyn FnMut()>);
                input
                    .add_event_listener_with_callback("input", cb.as_ref().unchecked_ref())
                    .unwrap();
                cb.forget();

                ParamCtrl::Slider(input)
            }
            ParamKind::Select(options) => {
                let sel: HtmlSelectElement = document
                    .create_element("select")
                    .unwrap()
                    .dyn_into()
                    .unwrap();
                select_style(&sel);
                for &(text, val) in options {
                    let opt = document.create_element("option").unwrap();
                    opt.set_text_content(Some(text));
                    opt.set_attribute("value", &val.to_string()).unwrap();
                    sel.append_child(&opt).unwrap();
                }
                let idx = options
                    .iter()
                    .position(|&(_, v)| (v - p.value).abs() < f64::EPSILON)
                    .unwrap_or(0);
                sel.set_selected_index(idx as i32);
                row.append_child(&sel).unwrap();

                if let Some(dirty) = dirty.cloned() {
                    let cb = Closure::wrap(Box::new(move || {
                        dirty.set(true);
                    }) as Box<dyn FnMut()>);
                    sel.add_event_listener_with_callback("change", cb.as_ref().unchecked_ref())
                        .unwrap();
                    cb.forget();
                }

                ParamCtrl::Select(sel)
            }
        };

        if let Some(before) = insert_before {
            container.insert_before(&row, Some(before)).unwrap();
        } else {
            container.append_child(&row).unwrap();
        }
        out.push((ctrl, val_span, p.name));
    }

    out
}
