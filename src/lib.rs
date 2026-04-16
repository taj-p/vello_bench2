//! WebGL benchmark tool for Vello Hybrid.
//!
//! Two modes:
//! - **Interactive** — tweak parameters in real-time, observe FPS.
//! - **Benchmark** — automated suite with warmup calibration, vsync-independent timing.

#![allow(
    clippy::cast_possible_truncation,
    reason = "truncation has no appreciable impact in this benchmark"
)]
#![cfg(target_arch = "wasm32")]

pub(crate) mod backend;
pub(crate) mod capability;
mod fps;
pub(crate) mod harness;
pub(crate) mod resource_store;
pub(crate) mod rng;
pub mod scenes;
pub(crate) mod storage;
pub mod ui;

use std::cell::RefCell;
use std::rc::Rc;

use backend::{
    Backend, BackendCapabilities, BackendKind, current_backend_capabilities, current_backend_kind,
    new_backend,
};
use fps::FpsTracker;
use harness::{
    BenchDef, BenchHarness, CalibrationEvent, CalibrationHarness, HarnessEvent, bench_defs,
};
use resource_store::ResourceStore;
use scenes::{BenchScene, scene_index};
use storage::CalibrationProfile;
use ui::{AppMode, Ui};
use vello_common::kurbo::Affine;
use wasm_bindgen::prelude::*;
use web_sys::{HtmlCanvasElement, HtmlIFrameElement};

type RafClosure = Rc<RefCell<Option<Closure<dyn FnMut()>>>>;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = requestAnimationFrame)]
    fn request_animation_frame(f: &Closure<dyn FnMut()>);
}

pub fn init_logging() {
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Info);
}

struct AppState {
    scenes: Vec<Box<dyn BenchScene>>,
    current_scene: usize,
    backend_caps: BackendCapabilities,
    backend: Box<dyn Backend>,
    canvas: HtmlCanvasElement,
    benchmark_canvas: HtmlCanvasElement,
    width: u32,
    height: u32,
    fps_tracker: FpsTracker,
    ui: Ui,
    harness: BenchHarness,
    calibration_harness: CalibrationHarness,
    ab_harness: Option<AbHarnessState>,
    bench_defs: Vec<BenchDef>,
    calibration: Option<CalibrationProfile>,
    resources: ResourceStore,
    // View state (pan in physical pixels, zoom multiplier).
    pan_x: f64,
    pan_y: f64,
    zoom: f64,
    dragging: bool,
    drag_last_x: f64,
    drag_last_y: f64,
    // Touch state for mobile pan/zoom.
    touch_count: u32,
    touch_last_x: f64,
    touch_last_y: f64,
    /// Distance between two fingers for pinch zoom.
    pinch_dist: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AbVariant {
    Control,
    Treatment,
}

#[derive(Debug)]
struct AbHarnessState {
    control_frame: HtmlIFrameElement,
    treatment_frame: HtmlIFrameElement,
    control_ready: bool,
    treatment_ready: bool,
    running: bool,
    selected: Vec<usize>,
    run_pos: usize,
    round_pos: usize,
    total_rounds: usize,
    control_samples: Vec<f64>,
    treatment_samples: Vec<f64>,
    pending_control: Option<harness::BenchResult>,
}

impl AbHarnessState {
    fn new(control_frame: HtmlIFrameElement, treatment_frame: HtmlIFrameElement) -> Self {
        Self {
            control_frame,
            treatment_frame,
            control_ready: false,
            treatment_ready: false,
            running: false,
            selected: Vec::new(),
            run_pos: 0,
            round_pos: 0,
            total_rounds: 1,
            control_samples: Vec::new(),
            treatment_samples: Vec::new(),
            pending_control: None,
        }
    }

    fn is_ready(&self) -> bool {
        self.control_ready && self.treatment_ready
    }
}

fn ab_mode_enabled() -> bool {
    js_sys::Reflect::get(&js_sys::global(), &"__vello_ab_mode".into())
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn parse_ab_variant(value: &str) -> Option<AbVariant> {
    match value {
        "control" => Some(AbVariant::Control),
        "treatment" => Some(AbVariant::Treatment),
        _ => None,
    }
}

fn current_simd_enabled() -> bool {
    js_sys::Reflect::get(&js_sys::global(), &"__vello_simd".into())
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("mode", &self.ui.mode)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl AppState {
    fn benchmark_running(&self) -> bool {
        self.harness.is_running()
            || self.calibration_harness.is_running()
            || self
                .ab_harness
                .as_ref()
                .is_some_and(|harness| harness.running)
    }

    fn scene_params_for_ui(&self, scene_idx: usize) -> Vec<scenes::Param> {
        scenes::visible_params(self.scenes[scene_idx].as_ref(), self.backend_caps)
    }

    fn calibration_key_for(&self, width: u32, height: u32) -> String {
        storage::calibration_key(
            self.backend.kind().as_str(),
            current_simd_enabled(),
            width,
            height,
        )
    }

    fn refresh_calibration_state(&mut self) {
        let (vp_w, vp_h) = self.ui.configured_viewport();
        if vp_w == 0 || vp_h == 0 {
            self.calibration = None;
            self.ui
                .set_calibration_status("Set a non-zero benchmark viewport to calibrate");
            self.ui.set_calibration_ready(false);
            self.ui.set_ab_ready(false);
            self.ui.update_bench_titles(&self.bench_defs, None);
            return;
        }

        let key = self.calibration_key_for(vp_w, vp_h);
        self.calibration = storage::load_calibration_profile(&key);
        self.ui
            .update_bench_titles(&self.bench_defs, self.calibration.as_ref());
        let ready = self.calibration.is_some();
        if ready {
            self.ui.set_calibration_status(&format!(
                "Calibrated for {} at {}x{}",
                self.backend.kind().label(),
                vp_w,
                vp_h
            ));
        } else {
            self.ui.set_calibration_status(&format!(
                "Calibration required for {} at {}x{}",
                self.backend.kind().label(),
                vp_w,
                vp_h
            ));
        }
        self.ui.set_calibration_ready(ready);
        let ab_ready = ready
            && self
                .ab_harness
                .as_ref()
                .is_some_and(AbHarnessState::is_ready);
        self.ui.set_ab_ready(ab_ready);
        self.harness.set_calibration(self.calibration.clone());
    }

    fn switch_backend(&mut self, kind: BackendKind, now: f64) -> bool {
        if self.backend.kind() == kind {
            return false;
        }

        crate::storage::save_backend_name(kind.as_str());
        self.dragging = false;
        self.resources.clear_all(self.backend.as_mut());

        let old_params = if self.ui.mode == AppMode::Interactive {
            self.ui.read_params()
        } else {
            Vec::new()
        };

        self.backend_caps = current_backend_capabilities(kind);
        self.canvas = replace_canvas_element(&self.canvas, self.width, self.height, self.ui.mode);
        self.backend = new_backend(&self.canvas, self.width, self.height, kind);
        self.scenes = scenes::all_scenes();

        let next_scene = if self
            .scenes
            .get(self.current_scene)
            .is_some_and(|scene| self.backend_caps.supports_scene(scene.scene_id()))
        {
            self.current_scene
        } else {
            scene_index(scenes::SceneId::Rect)
        };

        self.current_scene = next_scene;
        let scene_id = self.scenes[next_scene].scene_id();
        for (param_id, value) in old_params {
            if self.backend_caps.supports_param(scene_id, param_id)
                && self
                    .backend_caps
                    .supports_param_value(scene_id, param_id, value)
            {
                self.scenes[next_scene].set_param(param_id, value);
            }
        }

        self.ui.set_renderer(kind);
        self.ui
            .rebuild_scene_options(&self.scenes, self.backend_caps, self.current_scene);
        self.ui.update_bench_support(
            &self.bench_defs,
            self.calibration.as_ref(),
            &self.scenes,
            self.backend_caps,
        );
        let params = self.scene_params_for_ui(self.current_scene);
        self.ui.rebuild_params(&params);
        self.refresh_calibration_state();
        self.fps_tracker.reset(now);
        self.reset_view();
        self.ui.mark_dirty();
        true
    }

    fn reset_interactive_state(&mut self, now: f64) {
        self.dragging = false;
        self.touch_count = 0;
        self.pinch_dist = 0.0;
        self.resources.clear_all(self.backend.as_mut());
        let kind = self.backend.kind();
        self.backend = new_backend(&self.canvas, self.width, self.height, kind);
        self.scenes = scenes::all_scenes();
        let selected = self.ui.selected_scene();
        if selected < self.scenes.len()
            && self
                .backend_caps
                .supports_scene(self.scenes[selected].scene_id())
        {
            self.current_scene = selected;
        }
        for (param_id, value) in self.ui.read_params() {
            self.scenes[self.current_scene].set_param(param_id, value);
        }
        self.fps_tracker.reset(now);
        self.reset_view();
    }

    fn tick(&mut self, now: f64) {
        match self.ui.mode {
            AppMode::Interactive => self.tick_interactive(now),
            AppMode::Benchmark => self.tick_benchmark(now),
        }
        self.ui.flush_state();
    }

    fn tick_interactive(&mut self, now: f64) {
        let selected = self.ui.selected_scene();
        if selected != self.current_scene && selected < self.scenes.len() {
            let old_scene_id = self.scenes[self.current_scene].scene_id();
            self.resources
                .clear_scene(old_scene_id, self.backend.as_mut());
            self.current_scene = selected;
            let kind = self.backend.kind();
            self.backend = new_backend(&self.canvas, self.width, self.height, kind);
            self.scenes = scenes::all_scenes();
            self.fps_tracker.reset(now);
            self.reset_view();
            let params = self.scene_params_for_ui(self.current_scene);
            self.ui.rebuild_params(&params);
            self.ui.mark_dirty();
        }

        let params = self.ui.read_params();
        let idx = self.current_scene;
        for &(param_id, value) in &params {
            self.scenes[idx].set_param(param_id, value);
        }

        let perf = web_sys::window().unwrap().performance().unwrap();
        let t0 = perf.now();

        self.backend.reset();
        let (w, h) = (self.width, self.height);
        let view = Affine::translate((self.pan_x, self.pan_y)) * Affine::scale(self.zoom);
        self.scenes[idx].render(self.backend.as_mut(), &mut self.resources, w, h, now, view);

        let encode_ms = perf.now() - t0;

        self.backend.render_offscreen();
        let render_ms = perf.now() - t0 - encode_ms;

        self.backend.blit();
        let blit_ms = perf.now() - t0 - encode_ms - render_ms;

        let total_ms = perf.now() - t0;
        let (fps, frame_time) = self.fps_tracker.frame(now);
        let is_cpu = self.backend.is_cpu();
        let supports_encode_timing = self.backend.supports_encode_timing();
        self.ui.update_timing(
            fps,
            frame_time,
            encode_ms,
            render_ms,
            blit_ms,
            total_ms,
            is_cpu,
            supports_encode_timing,
        );
    }

    fn is_view_default(&self) -> bool {
        self.pan_x == 0.0 && self.pan_y == 0.0 && self.zoom == 1.0
    }

    fn update_reset_btn(&self) {
        let display = if self.is_view_default() {
            "none"
        } else {
            "flex"
        };
        self.ui
            .reset_view_btn
            .style()
            .set_property("display", display)
            .unwrap();
    }

    fn reset_view(&mut self) {
        self.pan_x = 0.0;
        self.pan_y = 0.0;
        self.zoom = 1.0;
        self.update_reset_btn();
    }

    /// Zoom centered on a point in physical pixels.
    fn zoom_at(&mut self, cx: f64, cy: f64, factor: f64) {
        let new_zoom = (self.zoom * factor).clamp(0.05, 100.0);
        let ratio = new_zoom / self.zoom;
        self.pan_x = cx - ratio * (cx - self.pan_x);
        self.pan_y = cy - ratio * (cy - self.pan_y);
        self.zoom = new_zoom;
        self.update_reset_btn();
    }

    fn tick_benchmark(&mut self, now: f64) {
        if self.calibration_harness.is_running() {
            if let Some(status) = self.calibration_harness.current_status() {
                self.ui.set_calibration_status(&status);
            }
            let events = self.calibration_harness.tick(self.width, self.height, now);
            for event in events {
                match event {
                    CalibrationEvent::AllDone(counts) => {
                        let key = self.calibration_key_for(self.width, self.height);
                        let profile = CalibrationProfile { key, counts };
                        storage::save_calibration_profile(profile.clone());
                        self.calibration = Some(profile);
                        self.harness.set_calibration(self.calibration.clone());
                        self.ui
                            .update_bench_titles(&self.bench_defs, self.calibration.as_ref());
                        self.ui.set_calibration_status(&format!(
                            "Calibration complete for {} at {}x{}",
                            self.backend.kind().label(),
                            self.width,
                            self.height
                        ));
                        self.ui.set_calibration_running(false);
                        self.ui.set_calibration_ready(true);
                        let ab_ready = self
                            .ab_harness
                            .as_ref()
                            .is_some_and(AbHarnessState::is_ready);
                        self.ui.set_ab_ready(ab_ready);
                        self.ui.set_benchmark_presentation(false);
                        set_canvas_visibility(&self.benchmark_canvas, false);
                        if let Some(document) = web_sys::window().and_then(|w| w.document()) {
                            set_stage_shell_presentation(&document, false);
                        }
                    }
                }
            }
            return;
        }

        if !self.harness.is_running() {
            return;
        }

        // Highlight the currently running bench row
        if let Some(idx) = self.harness.current_bench_idx() {
            self.ui.bench_set_running(idx);
            if let Some(def) = self.bench_defs.get(idx) {
                self.ui
                    .set_presentation_status(&format!("Running {}", def.name));
            }
        }

        let (w, h) = (self.width, self.height);
        let events = self.harness.tick(&self.bench_defs, w, h, now);

        for event in events {
            match event {
                HarnessEvent::BenchDone(ref result) => {
                    // Find which def index this result belongs to
                    if let Some(idx) = self.bench_defs.iter().position(|d| d.name == result.name) {
                        self.ui.bench_set_done(idx, result);
                    }
                }
                HarnessEvent::AllDone => {
                    self.ui.bench_all_done();
                    self.ui.set_calibration_ready(self.calibration.is_some());
                    self.ui.set_calibration_running(false);
                    let ab_ready = self.calibration.is_some()
                        && self
                            .ab_harness
                            .as_ref()
                            .is_some_and(AbHarnessState::is_ready);
                    self.ui.set_ab_ready(ab_ready);
                    self.ui.set_benchmark_presentation(false);
                    set_canvas_visibility(&self.benchmark_canvas, false);
                    if let Some(document) = web_sys::window().and_then(|w| w.document()) {
                        set_stage_shell_presentation(&document, false);
                    }
                }
            }
        }
    }

    fn start_ab_benchmark(&mut self) {
        if self.calibration.is_none() {
            self.ui
                .set_calibration_status("Calibration is required before running A/B benchmarks");
            return;
        }
        let ab_ready = self
            .ab_harness
            .as_ref()
            .is_some_and(AbHarnessState::is_ready);
        let ab_running = self.ab_harness.as_ref().is_some_and(|ab| ab.running);
        if !ab_ready || ab_running {
            return;
        }
        if self.ab_harness.is_none() {
            return;
        }

        let selected = self.ui.selected_bench_indices();
        if selected.is_empty() {
            return;
        }

        let (vp_w, vp_h) = self.ui.configured_viewport();
        if vp_w == 0 || vp_h == 0 {
            return;
        }

        self.width = vp_w;
        self.height = vp_h;

        self.ui.bench_started(&selected);
        self.ui.set_benchmark_presentation(true);
        if let Some(first_idx) = selected.first().copied()
            && let Some(def) = self.bench_defs.get(first_idx)
        {
            let status = format!(
                "Running control: {} ({}/{})",
                def.name,
                1,
                self.ui.ab_rounds()
            );
            self.ui.set_ab_status(&status);
            self.ui.set_presentation_status(&status);
        }
        set_canvas_visibility(&self.canvas, false);
        set_canvas_visibility(&self.benchmark_canvas, false);
        self.show_ab_variant(None);

        let ab = self.ab_harness.as_mut().unwrap();
        ab.running = true;
        ab.selected = selected;
        ab.run_pos = 0;
        ab.round_pos = 0;
        ab.total_rounds = self.ui.ab_rounds();
        ab.control_samples.clear();
        ab.treatment_samples.clear();
        ab.pending_control = None;

        self.send_ab_bench(AbVariant::Control);
    }

    fn start_calibration(&mut self) {
        if self.benchmark_running() {
            return;
        }
        let (vp_w, vp_h) = self.ui.configured_viewport();
        if vp_w == 0 || vp_h == 0 {
            self.ui
                .set_calibration_status("Set a non-zero benchmark viewport before calibrating");
            return;
        }

        self.width = vp_w;
        self.height = vp_h;
        self.canvas.set_width(vp_w);
        self.canvas.set_height(vp_h);
        self.benchmark_canvas.set_width(vp_w);
        self.benchmark_canvas.set_height(vp_h);
        self.backend.resize(vp_w, vp_h);
        self.harness.set_calibration(None);

        self.ui.set_calibration_running(true);
        self.ui.set_calibration_ready(false);
        self.ui.set_ab_ready(false);
        self.ui.set_calibration_status("Starting calibration…");
        self.ui.set_benchmark_presentation(true);
        set_canvas_visibility(&self.canvas, false);
        set_canvas_visibility(&self.benchmark_canvas, true);
        if let Some(document) = web_sys::window().and_then(|w| w.document()) {
            set_stage_shell_presentation(&document, true);
        }
        self.calibration_harness
            .start(&self.bench_defs, &self.benchmark_canvas, vp_w, vp_h);
    }

    fn show_ab_variant(&self, active: Option<AbVariant>) {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let host: web_sys::HtmlElement = document
            .get_element_by_id("ab-frame-host")
            .expect("ab-frame-host should exist in index.html")
            .dyn_into()
            .expect("ab-frame-host should be an HtmlElement");
        host.style()
            .set_property("display", if active.is_some() { "block" } else { "none" })
            .unwrap();
        if let Some(ab) = &self.ab_harness {
            ab.control_frame
                .style()
                .set_property(
                    "display",
                    if active == Some(AbVariant::Control) {
                        "block"
                    } else {
                        "none"
                    },
                )
                .unwrap();
            ab.treatment_frame
                .style()
                .set_property(
                    "display",
                    if active == Some(AbVariant::Treatment) {
                        "block"
                    } else {
                        "none"
                    },
                )
                .unwrap();
        }
    }

    fn send_ab_bench(&self, variant: AbVariant) {
        let Some(ab) = &self.ab_harness else {
            return;
        };
        let idx = ab.selected[ab.run_pos];
        let msg = js_sys::Object::new();
        js_sys::Reflect::set(&msg, &"type".into(), &"run_bench".into()).unwrap();
        js_sys::Reflect::set(&msg, &"idx".into(), &(idx as u32).into()).unwrap();
        js_sys::Reflect::set(
            &msg,
            &"warmup_samples".into(),
            &(self.ui.bench_warmup_samples() as u32).into(),
        )
        .unwrap();
        js_sys::Reflect::set(
            &msg,
            &"measured_samples".into(),
            &(self.ui.bench_measured_samples() as u32).into(),
        )
        .unwrap();
        js_sys::Reflect::set(&msg, &"width".into(), &self.width.into()).unwrap();
        js_sys::Reflect::set(&msg, &"height".into(), &self.height.into()).unwrap();
        js_sys::Reflect::set(
            &msg,
            &"backend".into(),
            &self.backend.kind().as_str().into(),
        )
        .unwrap();
        let target = match variant {
            AbVariant::Control => &ab.control_frame,
            AbVariant::Treatment => &ab.treatment_frame,
        };
        let _ = target.content_window().unwrap().post_message(&msg, "*");
    }

    fn handle_ab_ready(&mut self, variant: AbVariant) {
        let Some(ab) = self.ab_harness.as_mut() else {
            return;
        };
        match variant {
            AbVariant::Control => ab.control_ready = true,
            AbVariant::Treatment => ab.treatment_ready = true,
        }
        self.ui
            .set_ab_ready(ab.is_ready() && self.calibration.is_some());
        if ab.is_ready() {
            self.ui.set_ab_status("A/B harness ready");
        } else {
            self.ui.set_ab_status("Loading A/B runner…");
        }
    }

    fn handle_ab_bench_result(&mut self, variant: AbVariant, result: harness::BenchResult) {
        let Some(ab) = self.ab_harness.as_mut() else {
            return;
        };
        if !ab.running || ab.run_pos >= ab.selected.len() {
            return;
        }
        let idx = ab.selected[ab.run_pos];
        let bench_name = self
            .bench_defs
            .get(idx)
            .map(|def| def.name)
            .unwrap_or(result.name);
        match variant {
            AbVariant::Control => {
                ab.control_samples.push(result.ms_per_frame);
                let control_avg =
                    ab.control_samples.iter().sum::<f64>() / ab.control_samples.len() as f64;
                let control_preview = harness::BenchResult {
                    name: result.name,
                    ms_per_frame: control_avg,
                    iterations: result.iterations * ab.control_samples.len(),
                    total_ms: 0.0,
                };
                self.ui.bench_set_ab_control_done(idx, &control_preview);
                ab.pending_control = Some(result);
                let status = format!(
                    "Running treatment: {} ({}/{})",
                    bench_name,
                    ab.round_pos + 1,
                    ab.total_rounds
                );
                self.ui.set_ab_status(&status);
                self.ui.set_presentation_status(&status);
                self.show_ab_variant(None);
                self.send_ab_bench(AbVariant::Treatment);
            }
            AbVariant::Treatment => {
                let Some(control) = ab.pending_control.take() else {
                    return;
                };
                ab.treatment_samples.push(result.ms_per_frame);
                if ab.round_pos + 1 < ab.total_rounds {
                    ab.round_pos += 1;
                    let status = format!(
                        "Running control: {} ({}/{})",
                        bench_name,
                        ab.round_pos + 1,
                        ab.total_rounds
                    );
                    self.ui.set_ab_status(&status);
                    self.ui.set_presentation_status(&status);
                    self.show_ab_variant(None);
                    self.send_ab_bench(AbVariant::Control);
                } else {
                    let control_result = harness::BenchResult {
                        name: control.name,
                        ms_per_frame: ab.control_samples.iter().sum::<f64>()
                            / ab.control_samples.len() as f64,
                        iterations: control.iterations * ab.control_samples.len(),
                        total_ms: 0.0,
                    };
                    let treatment_result = harness::BenchResult {
                        name: result.name,
                        ms_per_frame: ab.treatment_samples.iter().sum::<f64>()
                            / ab.treatment_samples.len() as f64,
                        iterations: result.iterations * ab.treatment_samples.len(),
                        total_ms: 0.0,
                    };
                    self.ui
                        .bench_set_ab_done(idx, &control_result, &treatment_result);
                    ab.run_pos += 1;
                    ab.round_pos = 0;
                    ab.control_samples.clear();
                    ab.treatment_samples.clear();
                    if ab.run_pos < ab.selected.len() {
                        let next_idx = ab.selected[ab.run_pos];
                        let next_name = self
                            .bench_defs
                            .get(next_idx)
                            .map(|def| def.name)
                            .unwrap_or("benchmark");
                        let status =
                            format!("Running control: {} ({}/{})", next_name, 1, ab.total_rounds);
                        self.ui.set_ab_status(&status);
                        self.ui.set_presentation_status(&status);
                        self.show_ab_variant(None);
                        self.send_ab_bench(AbVariant::Control);
                    } else {
                        ab.running = false;
                        self.ui.bench_all_done();
                        self.ui.set_ab_status("A/B run complete");
                        self.ui.set_benchmark_presentation(false);
                        self.show_ab_variant(None);
                        if let Some(document) = web_sys::window().and_then(|w| w.document()) {
                            set_stage_shell_presentation(&document, false);
                        }
                    }
                }
            }
        }
    }
}

fn set_canvas_visibility(canvas: &HtmlCanvasElement, visible: bool) {
    canvas
        .style()
        .set_property("visibility", if visible { "visible" } else { "hidden" })
        .unwrap();
}

fn set_stage_shell_presentation(document: &web_sys::Document, active: bool) {
    let benchmark_shell: web_sys::HtmlElement = document
        .get_element_by_id("benchmark-stage-shell")
        .expect("benchmark-stage-shell should exist in index.html")
        .dyn_into()
        .expect("benchmark-stage-shell should be an HtmlElement");
    let style = benchmark_shell.style();
    style
        .set_property("display", if active { "block" } else { "none" })
        .unwrap();
    style
        .set_property("top", if active { "0" } else { "" })
        .unwrap();
    style
        .set_property("z-index", if active { "60" } else { "0" })
        .unwrap();
}

fn configure_canvas(canvas: &HtmlCanvasElement, px_w: u32, px_h: u32, mode: AppMode) {
    canvas.set_width(px_w);
    canvas.set_height(px_h);
    let cs = canvas.style();
    cs.set_property("position", "absolute").unwrap();
    cs.set_property("inset", "0").unwrap();
    cs.set_property("left", "0").unwrap();
    cs.set_property("z-index", "0").unwrap();
    cs.set_property("width", "100%").unwrap();
    cs.set_property("height", "100%").unwrap();
    cs.set_property("display", "block").unwrap();
    cs.set_property("touch-action", "none").unwrap();
    cs.set_property(
        "visibility",
        if mode == AppMode::Interactive {
            "visible"
        } else {
            "hidden"
        },
    )
    .unwrap();
}

fn stage_physical_size(document: &web_sys::Document) -> (u32, u32, u32, u32) {
    let stage = document
        .get_element_by_id("canvas-host")
        .expect("canvas-host should exist in index.html");
    let rect = stage.get_bounding_client_rect();
    let dpr = web_sys::window().unwrap().device_pixel_ratio();
    let css_w = rect.width().max(1.0).round() as u32;
    let css_h = rect.height().max(1.0).round() as u32;
    let px_w = (css_w as f64 * dpr).round() as u32;
    let px_h = (css_h as f64 * dpr).round() as u32;
    (css_w, css_h, px_w, px_h)
}

fn make_canvas(
    document: &web_sys::Document,
    px_w: u32,
    px_h: u32,
    mode: AppMode,
) -> HtmlCanvasElement {
    let canvas: HtmlCanvasElement = document
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    configure_canvas(&canvas, px_w, px_h, mode);
    canvas
}

fn replace_canvas_element(
    current: &HtmlCanvasElement,
    px_w: u32,
    px_h: u32,
    mode: AppMode,
) -> HtmlCanvasElement {
    let document = web_sys::window().unwrap().document().unwrap();
    let new_canvas = make_canvas(&document, px_w, px_h, mode);
    let parent = current.parent_node().unwrap();
    parent.insert_before(&new_canvas, Some(current)).unwrap();
    parent.remove_child(current).unwrap();
    new_canvas
}

fn client_to_canvas_px(canvas: &HtmlCanvasElement, client_x: f64, client_y: f64) -> (f64, f64) {
    let rect = canvas.get_bounding_client_rect();
    let width = rect.width().max(1.0);
    let height = rect.height().max(1.0);
    let x = ((client_x - rect.left()) / width).clamp(0.0, 1.0);
    let y = ((client_y - rect.top()) / height).clamp(0.0, 1.0);
    (x * canvas.width() as f64, y * canvas.height() as f64)
}

fn client_delta_to_canvas_px(canvas: &HtmlCanvasElement, delta_x: f64, delta_y: f64) -> (f64, f64) {
    let rect = canvas.get_bounding_client_rect();
    let scale_x = canvas.width() as f64 / rect.width().max(1.0);
    let scale_y = canvas.height() as f64 / rect.height().max(1.0);
    (delta_x * scale_x, delta_y * scale_y)
}

fn event_target_is_in_stage(target: &wasm_bindgen::JsValue) -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let Some(document) = window.document() else {
        return false;
    };
    let Some(stage) = document.get_element_by_id("canvas-host") else {
        return false;
    };
    let Ok(node) = target.clone().dyn_into::<web_sys::Node>() else {
        return false;
    };
    stage.contains(Some(&node))
}

/// Entry point.
pub async fn run() {
    init_logging();

    let window = web_sys::window().unwrap();
    let document = window.document().unwrap();
    let performance = window.performance().unwrap();
    let (_, _, px_w, px_h) = stage_physical_size(&document);

    let canvas = make_canvas(&document, px_w, px_h, AppMode::Interactive);
    let canvas_host = document
        .get_element_by_id("canvas-host")
        .expect("canvas-host should exist in index.html");
    canvas_host.append_child(&canvas).unwrap();

    let benchmark_canvas = make_canvas(&document, px_w, px_h, AppMode::Benchmark);
    let benchmark_canvas_host = document
        .get_element_by_id("benchmark-canvas-host")
        .expect("benchmark-canvas-host should exist in index.html");
    benchmark_canvas_host
        .append_child(&benchmark_canvas)
        .unwrap();

    let ab_harness = if ab_mode_enabled() {
        let frame_host: web_sys::HtmlElement = document
            .get_element_by_id("ab-frame-host")
            .expect("ab-frame-host should exist in index.html")
            .dyn_into()
            .expect("ab-frame-host should be an HtmlElement");

        let make_frame = |src: &str| -> HtmlIFrameElement {
            let frame: HtmlIFrameElement = document
                .create_element("iframe")
                .unwrap()
                .dyn_into()
                .unwrap();
            frame.set_src(src);
            frame
                .set_attribute("allow", "cross-origin-isolated")
                .unwrap();
            let style = frame.style();
            style.set_property("position", "absolute").unwrap();
            style.set_property("inset", "0").unwrap();
            style.set_property("width", "100%").unwrap();
            style.set_property("height", "100%").unwrap();
            style.set_property("border", "0").unwrap();
            style.set_property("display", "none").unwrap();
            frame
        };

        let control_frame = make_frame("control/ab_child.html");
        let treatment_frame = make_frame("treatment/ab_child.html");
        frame_host.append_child(&control_frame).unwrap();
        frame_host.append_child(&treatment_frame).unwrap();
        Some(AbHarnessState::new(control_frame, treatment_frame))
    } else {
        None
    };

    let bench_scenes = scenes::all_scenes();
    let defs = bench_defs();
    let backend_kind = current_backend_kind();
    let backend_caps = current_backend_capabilities(backend_kind);

    let saved_state = storage::load_ui_state();
    let initial_calibration = storage::load_calibration_profile(&storage::calibration_key(
        backend_kind.as_str(),
        current_simd_enabled(),
        px_w,
        px_h,
    ));
    let initial_mode = match saved_state.mode.as_deref() {
        Some("interactive") => AppMode::Interactive,
        _ => AppMode::Benchmark,
    };
    let initial_sidebar_collapsed = saved_state.sidebar_collapsed.unwrap_or(true);
    let initial_scene = saved_state
        .scene
        .filter(|&i| i < bench_scenes.len())
        .filter(|&i| backend_caps.supports_scene(bench_scenes[i].scene_id()))
        .or_else(|| Some(scene_index(scenes::SceneId::Rect)))
        .unwrap_or(0);

    let ui = Ui::build(
        &document,
        &bench_scenes,
        &defs,
        initial_calibration.as_ref(),
        backend_caps,
        initial_scene,
        initial_sidebar_collapsed,
        px_w,
        px_h,
    );
    let backend = new_backend(&canvas, px_w, px_h, backend_kind);
    let now = performance.now();

    configure_canvas(&canvas, px_w, px_h, initial_mode);
    configure_canvas(&benchmark_canvas, px_w, px_h, AppMode::Benchmark);
    set_stage_shell_presentation(&document, false);

    let state = Rc::new(RefCell::new(AppState {
        scenes: bench_scenes,
        current_scene: initial_scene,
        backend_caps,
        backend,
        canvas,
        benchmark_canvas,
        width: px_w,
        height: px_h,
        fps_tracker: FpsTracker::new(now),
        ui,
        harness: BenchHarness::new(),
        calibration_harness: CalibrationHarness::new(),
        ab_harness,
        bench_defs: defs,
        calibration: initial_calibration,
        resources: ResourceStore::new(),
        pan_x: 0.0,
        pan_y: 0.0,
        zoom: 1.0,
        dragging: false,
        drag_last_x: 0.0,
        drag_last_y: 0.0,
        touch_count: 0,
        touch_last_x: 0.0,
        touch_last_y: 0.0,
        pinch_dist: 0.0,
    }));

    {
        let mut st = state.borrow_mut();
        st.ui.set_mode(initial_mode);
        st.ui.set_benchmark_presentation(false);
        st.ui.apply_saved_benches(&saved_state);
        st.ui.apply_saved_bench_config(&saved_state);
        st.ui.apply_saved_params(&saved_state);
        st.refresh_calibration_state();
        st.ui.save_state();
    }

    wire_events(&state, &window);
}

/// Wire up all DOM event handlers.
fn wire_events(state: &Rc<RefCell<AppState>>, window: &web_sys::Window) {
    // Sidebar toggle
    {
        let s = state.clone();
        let btn = state.borrow().ui.toggle_btn().clone();
        let cb =
            Closure::wrap(Box::new(move || s.borrow_mut().ui.toggle_sidebar()) as Box<dyn FnMut()>);
        btn.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    // Mode tabs
    {
        let borrow = state.borrow();
        let (itab, btab) = borrow.ui.tab_elements();
        let itab = itab.clone();
        let btab = btab.clone();
        drop(borrow);

        let s = state.clone();
        let cb = Closure::wrap(Box::new(move || {
            let mut st = s.borrow_mut();
            if st.benchmark_running() {
                return;
            }
            let now = web_sys::window().unwrap().performance().unwrap().now();
            st.reset_interactive_state(now);
            st.ui.set_mode(AppMode::Interactive);
            st.ui.set_benchmark_presentation(false);
            st.ui.flush_state();
            set_canvas_visibility(&st.canvas, true);
            set_canvas_visibility(&st.benchmark_canvas, false);
            let document = web_sys::window().unwrap().document().unwrap();
            set_stage_shell_presentation(&document, false);
        }) as Box<dyn FnMut()>);
        itab.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();

        let s = state.clone();
        let cb = Closure::wrap(Box::new(move || {
            let mut st = s.borrow_mut();
            if st.benchmark_running() {
                return;
            }
            let now = web_sys::window().unwrap().performance().unwrap().now();
            st.reset_interactive_state(now);
            st.ui.set_mode(AppMode::Benchmark);
            st.ui.set_benchmark_presentation(false);
            st.refresh_calibration_state();
            st.ui.flush_state();
            set_canvas_visibility(&st.canvas, false);
            set_canvas_visibility(&st.benchmark_canvas, false);
            let document = web_sys::window().unwrap().document().unwrap();
            set_stage_shell_presentation(&document, false);
        }) as Box<dyn FnMut()>);
        btab.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    // Start benchmarks
    {
        let s = state.clone();
        let btn = state.borrow().ui.start_btn().clone();
        let cb = Closure::wrap(Box::new(move || {
            let mut st = s.borrow_mut();
            if st.benchmark_running() {
                return;
            }
            if st.calibration.is_none() {
                st.ui
                    .set_calibration_status("Calibration is required before running benchmarks");
                return;
            }
            let selected = st.ui.selected_bench_indices();
            if selected.is_empty() {
                return;
            }
            let (vp_w, vp_h) = st.ui.configured_viewport();
            if vp_w > 0 && vp_h > 0 && (vp_w != st.width || vp_h != st.height) {
                st.canvas.set_width(vp_w);
                st.canvas.set_height(vp_h);
                st.benchmark_canvas.set_width(vp_w);
                st.benchmark_canvas.set_height(vp_h);
                st.width = vp_w;
                st.height = vp_h;
                st.backend.resize(vp_w, vp_h);
            }
            st.harness.warmup_samples = st.ui.bench_warmup_samples();
            st.harness.measured_samples = st.ui.bench_measured_samples();
            st.ui.bench_started(&selected);
            if let Some(first_idx) = selected.first().copied()
                && let Some(def) = st.bench_defs.get(first_idx)
            {
                st.ui.set_presentation_status(&format!(
                    "Running {} (1/{})",
                    def.name,
                    selected.len()
                ));
            }
            st.ui.set_benchmark_presentation(true);
            set_canvas_visibility(&st.canvas, false);
            set_canvas_visibility(&st.benchmark_canvas, true);
            let document = web_sys::window().unwrap().document().unwrap();
            set_stage_shell_presentation(&document, true);
            let (w, h) = (st.width, st.height);
            let canvas = st.benchmark_canvas.clone();
            st.harness.start(selected, w, h, &canvas);
        }) as Box<dyn FnMut()>);
        btn.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    // Start calibration
    {
        let s = state.clone();
        let btn = state.borrow().ui.calibrate_btn().clone();
        let cb = Closure::wrap(Box::new(move || {
            s.borrow_mut().start_calibration();
        }) as Box<dyn FnMut()>);
        btn.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    // Start A/B benchmarks
    if let Some(btn) = state.borrow().ui.ab_start_btn().cloned() {
        let s = state.clone();
        let cb = Closure::wrap(Box::new(move || {
            let mut st = s.borrow_mut();
            if st.benchmark_running() {
                return;
            }
            let document = web_sys::window().unwrap().document().unwrap();
            set_stage_shell_presentation(&document, true);
            st.start_ab_benchmark();
        }) as Box<dyn FnMut()>);
        btn.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    // Save button
    {
        let s = state.clone();
        let btn = state.borrow().ui.save_btn().clone();
        let cb = Closure::wrap(Box::new(move || {
            s.borrow().ui.save_results();
        }) as Box<dyn FnMut()>);
        btn.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    // Load report dropdown
    {
        let s = state.clone();
        let sel = state.borrow().ui.load_select().clone();
        let cb = Closure::wrap(Box::new(move || {
            s.borrow_mut().ui.load_report_into_rows();
        }) as Box<dyn FnMut()>);
        sel.add_event_listener_with_callback("change", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    // Compare dropdown
    {
        let s = state.clone();
        let sel = state.borrow().ui.compare_select().clone();
        let cb = Closure::wrap(Box::new(move || {
            s.borrow_mut().ui.load_comparison();
        }) as Box<dyn FnMut()>);
        sel.add_event_listener_with_callback("change", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    // Delete report button
    {
        let s = state.clone();
        let btn = state.borrow().ui.delete_btn.clone();
        let cb = Closure::wrap(Box::new(move || {
            s.borrow_mut().ui.delete_selected_report();
        }) as Box<dyn FnMut()>);
        btn.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    // Reset view button
    {
        let s = state.clone();
        let btn = state.borrow().ui.reset_view_btn.clone();
        let cb = Closure::wrap(Box::new(move || {
            s.borrow_mut().reset_view();
        }) as Box<dyn FnMut()>);
        btn.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    // Scene select → mark dirty
    {
        let dirty = state.borrow().ui.dirty_flag();
        let sel = state.borrow().ui.scene_select.clone();
        let cb = Closure::wrap(Box::new(move || {
            dirty.set(true);
        }) as Box<dyn FnMut()>);
        sel.add_event_listener_with_callback("change", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    // Backend select → switch backend at runtime.
    {
        let s = state.clone();
        let select = state.borrow().ui.renderer_select().clone();
        let select_for_cb = select.clone();
        let cb = Closure::wrap(Box::new(move || {
            let mut st = s.borrow_mut();
            if st.benchmark_running() {
                st.ui.set_renderer(st.backend.kind());
                return;
            }
            let Some(kind) = BackendKind::from_str(&select_for_cb.value()) else {
                st.ui.set_renderer(st.backend.kind());
                return;
            };
            let now = web_sys::window().unwrap().performance().unwrap().now();
            let replaced_canvas = st.switch_backend(kind, now);
            drop(st);
            if replaced_canvas {
                wire_touch(&s);
            }
        }) as Box<dyn FnMut()>);
        select
            .add_event_listener_with_callback("change", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    // Bench checkbox changes → mark dirty
    {
        let checkboxes: Vec<web_sys::HtmlInputElement> = state
            .borrow()
            .ui
            .bench_checkbox_elements()
            .into_iter()
            .cloned()
            .collect();
        for cb_el in checkboxes {
            let s = state.clone();
            let cb = Closure::wrap(Box::new(move || {
                let st = s.borrow_mut();
                st.ui.mark_dirty();
                st.ui.sync_bench_checkbox_state();
            }) as Box<dyn FnMut()>);
            cb_el
                .add_event_listener_with_callback("change", cb.as_ref().unchecked_ref())
                .unwrap();
            cb.forget();
        }
    }

    // Benchmark sample config changes → mark dirty
    for input in [
        state.borrow().ui.warmup_input().clone(),
        state.borrow().ui.measured_input().clone(),
    ] {
        let dirty = state.borrow().ui.dirty_flag();
        let cb = Closure::wrap(Box::new(move || {
            dirty.set(true);
        }) as Box<dyn FnMut()>);
        input
            .add_event_listener_with_callback("input", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    if let Some(input) = state.borrow().ui.ab_rounds_input().cloned() {
        let dirty = state.borrow().ui.dirty_flag();
        let cb = Closure::wrap(Box::new(move || {
            dirty.set(true);
        }) as Box<dyn FnMut()>);
        input
            .add_event_listener_with_callback("input", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    for input in [
        state.borrow().ui.vp_width_input().clone(),
        state.borrow().ui.vp_height_input().clone(),
    ] {
        let s = state.clone();
        let dirty = state.borrow().ui.dirty_flag();
        let cb = Closure::wrap(Box::new(move || {
            dirty.set(true);
            s.borrow_mut().refresh_calibration_state();
        }) as Box<dyn FnMut()>);
        input
            .add_event_listener_with_callback("input", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    wire_pan_zoom(state, window);
    wire_touch(state);
    wire_animation_loop(state);
    wire_resize(state);
    wire_ab_messages(state);
}

/// Wire pan (mouse drag) and zoom (wheel/pinch) on the window.
fn wire_pan_zoom(state: &Rc<RefCell<AppState>>, window: &web_sys::Window) {
    let s = state.clone();
    let cb = Closure::wrap(Box::new(move |e: web_sys::MouseEvent| {
        let Ok(mut st) = s.try_borrow_mut() else { return };
        if st.ui.mode != AppMode::Interactive {
            return;
        }
        if let Some(target) = e.target() {
            if !event_target_is_in_stage(&target) {
                return;
            }
        }
        st.dragging = true;
        st.drag_last_x = e.client_x() as f64;
        st.drag_last_y = e.client_y() as f64;
    }) as Box<dyn FnMut(_)>);
    window
        .add_event_listener_with_callback("mousedown", cb.as_ref().unchecked_ref())
        .unwrap();
    cb.forget();

    let s = state.clone();
    let cb = Closure::wrap(Box::new(move |e: web_sys::MouseEvent| {
        let Ok(mut st) = s.try_borrow_mut() else { return };
        if !st.dragging {
            return;
        }
        let x = e.client_x() as f64;
        let y = e.client_y() as f64;
        let (dx, dy) =
            client_delta_to_canvas_px(&st.canvas, x - st.drag_last_x, y - st.drag_last_y);
        st.pan_x += dx;
        st.pan_y += dy;
        st.drag_last_x = x;
        st.drag_last_y = y;
        st.update_reset_btn();
    }) as Box<dyn FnMut(_)>);
    window
        .add_event_listener_with_callback("mousemove", cb.as_ref().unchecked_ref())
        .unwrap();
    cb.forget();

    let s = state.clone();
    let cb = Closure::wrap(Box::new(move |_: web_sys::MouseEvent| {
        if let Ok(mut st) = s.try_borrow_mut() {
            st.dragging = false;
        }
    }) as Box<dyn FnMut(_)>);
    window
        .add_event_listener_with_callback("mouseup", cb.as_ref().unchecked_ref())
        .unwrap();
    cb.forget();

    let s = state.clone();
    let cb = Closure::wrap(Box::new(move |e: web_sys::WheelEvent| {
        let Ok(mut st) = s.try_borrow_mut() else { return };
        if st.ui.mode != AppMode::Interactive {
            return;
        }
        e.prevent_default();
        let (cx, cy) = client_to_canvas_px(&st.canvas, e.client_x() as f64, e.client_y() as f64);

        let dy = e.delta_y();
        let scale = if e.ctrl_key() {
            0.01
        } else {
            let line_mult = if e.delta_mode() == 1 { 16.0 } else { 1.0 };
            0.002 * line_mult
        };
        let factor = (-dy * scale).exp();
        st.zoom_at(cx, cy, factor);
    }) as Box<dyn FnMut(_)>);
    let opts = web_sys::AddEventListenerOptions::new();
    opts.set_passive(false);
    window
        .add_event_listener_with_callback_and_add_event_listener_options(
            "wheel",
            cb.as_ref().unchecked_ref(),
            &opts,
        )
        .unwrap();
    cb.forget();
}

/// Helper: compute distance between two touches.
fn touch_distance(t: &web_sys::TouchList) -> f64 {
    if t.length() < 2 {
        return 0.0;
    }
    let a = t.get(0).unwrap();
    let b = t.get(1).unwrap();
    let dx = (a.client_x() - b.client_x()) as f64;
    let dy = (a.client_y() - b.client_y()) as f64;
    (dx * dx + dy * dy).sqrt()
}

/// Helper: compute midpoint of two touches (in client coords).
fn touch_midpoint(t: &web_sys::TouchList) -> (f64, f64) {
    if t.length() < 2 {
        let a = t.get(0).unwrap();
        return (a.client_x() as f64, a.client_y() as f64);
    }
    let a = t.get(0).unwrap();
    let b = t.get(1).unwrap();
    (
        (a.client_x() + b.client_x()) as f64 * 0.5,
        (a.client_y() + b.client_y()) as f64 * 0.5,
    )
}

/// Wire touch events for mobile pan (1 finger) and pinch-to-zoom (2 fingers).
fn wire_touch(state: &Rc<RefCell<AppState>>) {
    let canvas = state.borrow().canvas.clone();
    let target: &web_sys::EventTarget = canvas.as_ref();
    let opts = web_sys::AddEventListenerOptions::new();
    opts.set_passive(false);

    // touchstart
    {
        let s = state.clone();
        let cb = Closure::wrap(Box::new(move |e: web_sys::TouchEvent| {
            let mut st = s.borrow_mut();
            if st.ui.mode != AppMode::Interactive {
                return;
            }
            e.prevent_default();
            let touches = e.touches();
            st.touch_count = touches.length();
            if touches.length() == 1 {
                let t = touches.get(0).unwrap();
                st.touch_last_x = t.client_x() as f64;
                st.touch_last_y = t.client_y() as f64;
            } else if touches.length() >= 2 {
                st.pinch_dist = touch_distance(&touches);
                let (mx, my) = touch_midpoint(&touches);
                st.touch_last_x = mx;
                st.touch_last_y = my;
            }
        }) as Box<dyn FnMut(_)>);
        target
            .add_event_listener_with_callback_and_add_event_listener_options(
                "touchstart",
                cb.as_ref().unchecked_ref(),
                &opts,
            )
            .unwrap();
        cb.forget();
    }

    // touchmove
    {
        let s = state.clone();
        let cb = Closure::wrap(Box::new(move |e: web_sys::TouchEvent| {
            let mut st = s.borrow_mut();
            if st.ui.mode != AppMode::Interactive || st.touch_count == 0 {
                return;
            }
            e.prevent_default();
            let touches = e.touches();

            if touches.length() == 1 && st.touch_count == 1 {
                // Single finger pan.
                let t = touches.get(0).unwrap();
                let x = t.client_x() as f64;
                let y = t.client_y() as f64;
                let (dx, dy) =
                    client_delta_to_canvas_px(&st.canvas, x - st.touch_last_x, y - st.touch_last_y);
                st.pan_x += dx;
                st.pan_y += dy;
                st.touch_last_x = x;
                st.touch_last_y = y;
                st.update_reset_btn();
            } else if touches.length() >= 2 {
                // Pinch zoom + two-finger pan.
                let new_dist = touch_distance(&touches);
                let (mx, my) = touch_midpoint(&touches);

                if st.pinch_dist > 0.0 {
                    let factor = new_dist / st.pinch_dist;
                    let (cx, cy) = client_to_canvas_px(&st.canvas, mx, my);
                    st.zoom_at(cx, cy, factor);
                }
                // Pan by midpoint delta.
                let (dx, dy) = client_delta_to_canvas_px(
                    &st.canvas,
                    mx - st.touch_last_x,
                    my - st.touch_last_y,
                );
                st.pan_x += dx;
                st.pan_y += dy;

                st.pinch_dist = new_dist;
                st.touch_last_x = mx;
                st.touch_last_y = my;
                st.touch_count = touches.length();
            }
        }) as Box<dyn FnMut(_)>);
        target
            .add_event_listener_with_callback_and_add_event_listener_options(
                "touchmove",
                cb.as_ref().unchecked_ref(),
                &opts,
            )
            .unwrap();
        cb.forget();
    }

    // touchend / touchcancel
    {
        let s = state.clone();
        let cb = Closure::wrap(Box::new(move |e: web_sys::TouchEvent| {
            let mut st = s.borrow_mut();
            let touches = e.touches();
            st.touch_count = touches.length();
            if touches.length() == 1 {
                // Went from 2→1 finger: reset single-finger tracking.
                let t = touches.get(0).unwrap();
                st.touch_last_x = t.client_x() as f64;
                st.touch_last_y = t.client_y() as f64;
            }
            if touches.length() == 0 {
                st.pinch_dist = 0.0;
            }
        }) as Box<dyn FnMut(_)>);
        target
            .add_event_listener_with_callback("touchend", cb.as_ref().unchecked_ref())
            .unwrap();
        target
            .add_event_listener_with_callback("touchcancel", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }
}

/// Start the requestAnimationFrame loop.
fn wire_animation_loop(state: &Rc<RefCell<AppState>>) {
    let f: RafClosure = Rc::new(RefCell::new(None));
    let g = f.clone();
    let s = state.clone();
    *g.borrow_mut() = Some(Closure::wrap(Box::new(move || {
        let now = web_sys::window().unwrap().performance().unwrap().now();
        if let Ok(mut st) = s.try_borrow_mut() {
            st.tick(now);
        }
        request_animation_frame(f.borrow().as_ref().unwrap());
    }) as Box<dyn FnMut()>));
    request_animation_frame(g.borrow().as_ref().unwrap());
}

/// Handle window resize events.
fn wire_resize(state: &Rc<RefCell<AppState>>) {
    let s = state.clone();
    let cb = Closure::wrap(Box::new(move |_: web_sys::Event| {
        let mut st = s.borrow_mut();
        let document = web_sys::window().unwrap().document().unwrap();
        let (_, _, px_w, px_h) = stage_physical_size(&document);
        st.canvas.set_width(px_w);
        st.canvas.set_height(px_h);
        st.benchmark_canvas.set_width(px_w);
        st.benchmark_canvas.set_height(px_h);
        st.width = px_w;
        st.height = px_h;
        st.backend.resize(px_w, px_h);
        st.ui.update_viewport(px_w, px_h);
    }) as Box<dyn FnMut(_)>);
    web_sys::window()
        .unwrap()
        .add_event_listener_with_callback("resize", cb.as_ref().unchecked_ref())
        .unwrap();
    cb.forget();
}

fn wire_ab_messages(state: &Rc<RefCell<AppState>>) {
    if !ab_mode_enabled() {
        return;
    }

    let s = state.clone();
    let cb = Closure::wrap(Box::new(move |event: web_sys::MessageEvent| {
        let data = event.data();
        let Some(obj) = data.dyn_ref::<js_sys::Object>() else {
            return;
        };
        let msg_type = js_sys::Reflect::get(obj, &"type".into())
            .ok()
            .and_then(|v| v.as_string());
        let variant = js_sys::Reflect::get(obj, &"variant".into())
            .ok()
            .and_then(|v| v.as_string())
            .and_then(|s| parse_ab_variant(&s));
        let Some(variant) = variant else {
            return;
        };

        let mut st = s.borrow_mut();
        match msg_type.as_deref() {
            Some("ready") => st.handle_ab_ready(variant),
            Some("bench_started") => st.show_ab_variant(Some(variant)),
            Some("bench_result") => {
                let name = js_sys::Reflect::get(obj, &"name".into())
                    .ok()
                    .and_then(|v| v.as_string());
                let ms_per_frame = js_sys::Reflect::get(obj, &"ms_per_frame".into())
                    .ok()
                    .and_then(|v| v.as_f64());
                let iterations = js_sys::Reflect::get(obj, &"iterations".into())
                    .ok()
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as usize;
                let total_ms = js_sys::Reflect::get(obj, &"total_ms".into())
                    .ok()
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let (Some(name), Some(ms_per_frame)) = (name, ms_per_frame) else {
                    return;
                };
                st.handle_ab_bench_result(
                    variant,
                    harness::BenchResult {
                        name: Box::leak(name.into_boxed_str()),
                        ms_per_frame,
                        iterations,
                        total_ms,
                    },
                );
            }
            _ => {}
        }
    }) as Box<dyn FnMut(_)>);
    web_sys::window()
        .unwrap()
        .add_event_listener_with_callback("message", cb.as_ref().unchecked_ref())
        .unwrap();
    cb.forget();
}

#[derive(Debug)]
struct AbChildState {
    canvas: HtmlCanvasElement,
    harness: BenchHarness,
    defs: Vec<BenchDef>,
}

#[wasm_bindgen]
pub fn ab_child_init() {
    init_logging();

    let window = web_sys::window().unwrap();
    let document = window.document().unwrap();
    let body = document.body().unwrap();
    body.set_class_name("m-0 overflow-hidden bg-slate-950");

    let canvas = make_canvas(&document, 1, 1, AppMode::Benchmark);
    let style = canvas.style();
    style.set_property("position", "fixed").unwrap();
    style.set_property("inset", "0").unwrap();
    style.set_property("width", "100%").unwrap();
    style.set_property("height", "100%").unwrap();
    style.set_property("visibility", "visible").unwrap();
    body.append_child(&canvas).unwrap();

    let state = Rc::new(RefCell::new(AbChildState {
        canvas: canvas.clone(),
        harness: BenchHarness::new(),
        defs: bench_defs(),
    }));

    {
        let state = state.clone();
        let cb = Closure::wrap(Box::new(move |event: web_sys::MessageEvent| {
            let data = event.data();
            let Some(obj) = data.dyn_ref::<js_sys::Object>() else {
                return;
            };
            let msg_type = js_sys::Reflect::get(obj, &"type".into())
                .ok()
                .and_then(|v| v.as_string());
            if msg_type.as_deref() != Some("run_bench") {
                return;
            }

            let get_f64 = |key: &str| {
                js_sys::Reflect::get(obj, &key.into())
                    .ok()
                    .and_then(|v| v.as_f64())
            };
            let idx = get_f64("idx").unwrap_or(0.0) as usize;
            let warmup_samples = get_f64("warmup_samples").unwrap_or(3.0) as usize;
            let measured_samples = get_f64("measured_samples").unwrap_or(15.0) as usize;
            let width = get_f64("width").unwrap_or(800.0) as u32;
            let height = get_f64("height").unwrap_or(600.0) as u32;
            let backend = js_sys::Reflect::get(obj, &"backend".into())
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_else(|| current_backend_kind().as_str().to_string());

            let mut st = state.borrow_mut();
            st.canvas.set_width(width);
            st.canvas.set_height(height);
            st.harness.warmup_samples = warmup_samples;
            st.harness.measured_samples = measured_samples.max(1);
            st.harness
                .set_calibration(storage::load_calibration_profile(
                    &storage::calibration_key(&backend, current_simd_enabled(), width, height),
                ));
            let canvas = st.canvas.clone();
            st.harness.start(vec![idx], width, height, &canvas);
            drop(st);

            let started = js_sys::Object::new();
            js_sys::Reflect::set(&started, &"type".into(), &"bench_started".into()).unwrap();
            js_sys::Reflect::set(
                &started,
                &"variant".into(),
                &js_sys::Reflect::get(&js_sys::global(), &"__vello_variant".into())
                    .unwrap_or(JsValue::NULL),
            )
            .unwrap();
            let _ = web_sys::window()
                .unwrap()
                .parent()
                .unwrap()
                .unwrap()
                .post_message(&started, "*");
        }) as Box<dyn FnMut(_)>);
        window
            .add_event_listener_with_callback("message", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    {
        let f: RafClosure = Rc::new(RefCell::new(None));
        let g = f.clone();
        let state = state.clone();
        *g.borrow_mut() = Some(Closure::wrap(Box::new(move || {
            let now = web_sys::window().unwrap().performance().unwrap().now();
            let mut st = state.borrow_mut();
            let width = st.canvas.width();
            let height = st.canvas.height();
            let defs = st.defs.clone();
            let events = st.harness.tick(&defs, width, height, now);
            for event in events {
                if let HarnessEvent::BenchDone(result) = event {
                    let reply = js_sys::Object::new();
                    js_sys::Reflect::set(&reply, &"type".into(), &"bench_result".into()).unwrap();
                    js_sys::Reflect::set(
                        &reply,
                        &"variant".into(),
                        &js_sys::Reflect::get(&js_sys::global(), &"__vello_variant".into())
                            .unwrap_or(JsValue::NULL),
                    )
                    .unwrap();
                    js_sys::Reflect::set(&reply, &"name".into(), &result.name.into()).unwrap();
                    js_sys::Reflect::set(
                        &reply,
                        &"ms_per_frame".into(),
                        &result.ms_per_frame.into(),
                    )
                    .unwrap();
                    js_sys::Reflect::set(
                        &reply,
                        &"iterations".into(),
                        &(result.iterations as u32).into(),
                    )
                    .unwrap();
                    js_sys::Reflect::set(&reply, &"total_ms".into(), &result.total_ms.into())
                        .unwrap();
                    let _ = web_sys::window()
                        .unwrap()
                        .parent()
                        .unwrap()
                        .unwrap()
                        .post_message(&reply, "*");
                }
            }
            request_animation_frame(f.borrow().as_ref().unwrap());
        }) as Box<dyn FnMut()>));
        request_animation_frame(g.borrow().as_ref().unwrap());
    }

    let ready = js_sys::Object::new();
    js_sys::Reflect::set(&ready, &"type".into(), &"ready".into()).unwrap();
    js_sys::Reflect::set(
        &ready,
        &"variant".into(),
        &js_sys::Reflect::get(&js_sys::global(), &"__vello_variant".into())
            .unwrap_or(JsValue::NULL),
    )
    .unwrap();
    let _ = window.parent().unwrap().unwrap().post_message(&ready, "*");
}
