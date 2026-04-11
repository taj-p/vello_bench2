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
pub(crate) mod rng;
pub mod scenes;
pub(crate) mod storage;
pub mod ui;

use std::cell::RefCell;
use std::rc::Rc;

use backend::{
    Backend, BackendCapabilities, BackendKind, current_backend_capabilities, current_backend_kind,
};
use fps::FpsTracker;
use harness::{BenchDef, BenchHarness, HarnessEvent, bench_defs};
use scenes::{BenchScene, scene_index};
use ui::{AppMode, Ui};
use vello_common::kurbo::Affine;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

type RafClosure = Rc<RefCell<Option<Closure<dyn FnMut()>>>>;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = requestAnimationFrame)]
    fn request_animation_frame(f: &Closure<dyn FnMut()>);
}

struct AppState {
    scenes: Vec<Box<dyn BenchScene>>,
    current_scene: usize,
    backend_caps: BackendCapabilities,
    backend: Backend,
    canvas: HtmlCanvasElement,
    width: u32,
    height: u32,
    fps_tracker: FpsTracker,
    ui: Ui,
    harness: BenchHarness,
    bench_defs: Vec<BenchDef>,
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
    fn scene_params_for_ui(&self, scene_idx: usize) -> Vec<scenes::Param> {
        scenes::visible_params(self.scenes[scene_idx].as_ref(), self.backend_caps)
    }

    fn switch_backend(&mut self, kind: BackendKind, now: f64) -> bool {
        if self.backend.kind() == kind {
            return false;
        }

        crate::storage::save_backend_name(kind.as_str());

        let old_params = if self.ui.mode == AppMode::Interactive {
            self.ui.read_params()
        } else {
            Vec::new()
        };

        self.backend_caps = current_backend_capabilities(kind);
        self.canvas = replace_canvas_element(&self.canvas, self.width, self.height, self.ui.mode);
        self.backend = Backend::new(&self.canvas, self.width, self.height, kind);
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
        self.ui
            .update_bench_support(&self.bench_defs, &self.scenes, self.backend_caps);
        let params = self.scene_params_for_ui(self.current_scene);
        self.ui.rebuild_params(&params);
        self.fps_tracker.reset(now);
        self.reset_view();
        self.ui.mark_dirty();
        true
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
            self.current_scene = selected;
            let kind = self.backend.kind();
            self.backend = Backend::new(&self.canvas, self.width, self.height, kind);
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
        self.scenes[idx].render(&mut self.backend, w, h, now, view);

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
            "block"
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

    fn tick_benchmark(&mut self, _now: f64) {
        if !self.harness.is_running() {
            return;
        }

        // Highlight the currently running bench row
        if let Some(idx) = self.harness.current_bench_idx() {
            self.ui.bench_set_running(idx);
        }

        let (w, h) = (self.width, self.height);
        let events = self.harness.tick(&self.bench_defs, w, h);

        for event in events {
            match event {
                HarnessEvent::ScreenshotReady => {
                    if let (Some(idx), Ok(url)) =
                        (self.harness.current_bench_idx(), self.canvas.to_data_url())
                    {
                        self.ui.set_screenshot(idx, &url);
                    }
                }
                HarnessEvent::BenchDone(ref result) => {
                    // Find which def index this result belongs to
                    if let Some(idx) = self.bench_defs.iter().position(|d| d.name == result.name) {
                        self.ui.bench_set_done(idx, result);
                    }
                }
                HarnessEvent::AllDone => {
                    self.ui.bench_all_done();
                }
            }
        }
    }
}

pub(crate) fn gpu_sync(renderer: &vello_hybrid::WebGlRenderer) {
    let gl = renderer.gl_context();
    let mut pixel = [0_u8; 4];
    gl.read_pixels_with_opt_u8_array(
        0,
        0,
        1,
        1,
        web_sys::WebGl2RenderingContext::RGBA,
        web_sys::WebGl2RenderingContext::UNSIGNED_BYTE,
        Some(&mut pixel),
    )
    .unwrap();
}

fn configure_canvas(canvas: &HtmlCanvasElement, px_w: u32, px_h: u32, mode: AppMode) {
    let window = web_sys::window().unwrap();
    let css_w = window.inner_width().unwrap().as_f64().unwrap() as u32;
    let css_h = window.inner_height().unwrap().as_f64().unwrap() as u32;
    canvas.set_width(px_w);
    canvas.set_height(px_h);
    let cs = canvas.style();
    cs.set_property("position", "fixed").unwrap();
    cs.set_property("top", "40px").unwrap();
    cs.set_property("left", "0").unwrap();
    cs.set_property("z-index", "0").unwrap();
    cs.set_property("width", &format!("{css_w}px")).unwrap();
    cs.set_property("height", &format!("{}px", css_h.saturating_sub(40)))
        .unwrap();
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

/// Entry point.
pub async fn run() {
    let window = web_sys::window().unwrap();
    let document = window.document().unwrap();
    let performance = window.performance().unwrap();
    let dpr = window.device_pixel_ratio();

    let css_w = window.inner_width().unwrap().as_f64().unwrap() as u32;
    let css_h = window.inner_height().unwrap().as_f64().unwrap() as u32;
    let px_w = (css_w as f64 * dpr) as u32;
    let px_h = (css_h as f64 * dpr) as u32;

    let canvas = make_canvas(&document, px_w, px_h, AppMode::Interactive);
    document.body().unwrap().append_child(&canvas).unwrap();

    let bench_scenes = scenes::all_scenes();
    let defs = bench_defs();
    let backend_kind = current_backend_kind();
    let backend_caps = current_backend_capabilities(backend_kind);

    let saved_state = storage::load_ui_state();
    let initial_mode = match saved_state.mode.as_deref() {
        Some("interactive") => AppMode::Interactive,
        _ => AppMode::Benchmark,
    };
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
        backend_caps,
        initial_scene,
        px_w,
        px_h,
    );
    let backend = Backend::new(&canvas, px_w, px_h, backend_kind);
    let now = performance.now();

    configure_canvas(&canvas, px_w, px_h, initial_mode);

    let state = Rc::new(RefCell::new(AppState {
        scenes: bench_scenes,
        current_scene: initial_scene,
        backend_caps,
        backend,
        canvas,
        width: px_w,
        height: px_h,
        fps_tracker: FpsTracker::new(now),
        ui,
        harness: BenchHarness::new(),
        bench_defs: defs,
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
        st.ui.apply_saved_benches(&saved_state);
        st.ui.apply_saved_bench_preset(&saved_state);
        st.ui.apply_saved_params(&saved_state);
        st.ui.load_ab_comparison();
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
            st.ui.set_mode(AppMode::Interactive);
            st.ui.flush_state();
            st.canvas
                .style()
                .set_property("visibility", "visible")
                .unwrap();
        }) as Box<dyn FnMut()>);
        itab.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();

        let s = state.clone();
        let cb = Closure::wrap(Box::new(move || {
            let mut st = s.borrow_mut();
            st.ui.set_mode(AppMode::Benchmark);
            st.ui.flush_state();
            st.canvas
                .style()
                .set_property("visibility", "hidden")
                .unwrap();
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
            let selected = st.ui.selected_bench_indices();
            if selected.is_empty() {
                return;
            }
            let (vp_w, vp_h) = st.ui.configured_viewport();
            if vp_w > 0 && vp_h > 0 && (vp_w != st.width || vp_h != st.height) {
                st.canvas.set_width(vp_w);
                st.canvas.set_height(vp_h);
                st.width = vp_w;
                st.height = vp_h;
                st.backend.resize(vp_w, vp_h);
            }
            st.harness.warmup_ms = st.ui.warmup_ms();
            st.harness.run_ms = st.ui.run_ms();
            st.harness.preset = st.ui.bench_preset();
            st.ui.bench_started(&selected);
            let (w, h) = (st.width, st.height);
            let canvas = st.canvas.clone();
            st.harness.start(selected, w, h, &canvas);
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
            if st.harness.is_running() {
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
            let dirty = state.borrow().ui.dirty_flag();
            let cb = Closure::wrap(Box::new(move || {
                dirty.set(true);
            }) as Box<dyn FnMut()>);
            cb_el
                .add_event_listener_with_callback("change", cb.as_ref().unchecked_ref())
                .unwrap();
            cb.forget();
        }
    }

    // Preset changes → mark dirty
    {
        let input = state.borrow().ui.preset_input().clone();
        let s = state.clone();
        let cb = Closure::wrap(Box::new(move || {
            let st = s.borrow_mut();
            st.ui.update_bench_titles();
            st.ui.mark_dirty();
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
}

/// Wire pan (mouse drag) and zoom (wheel/pinch) on the window.
fn wire_pan_zoom(state: &Rc<RefCell<AppState>>, window: &web_sys::Window) {
    let s = state.clone();
    let sidebar = state.borrow().ui.sidebar().clone();
    let cb = Closure::wrap(Box::new(move |e: web_sys::MouseEvent| {
        let mut st = s.borrow_mut();
        if st.ui.mode != AppMode::Interactive {
            return;
        }
        if let Some(target) = e.target() {
            if let Ok(node) = target.dyn_into::<web_sys::Node>() {
                if sidebar.contains(Some(&node)) {
                    return;
                }
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
        let mut st = s.borrow_mut();
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
        s.borrow_mut().dragging = false;
    }) as Box<dyn FnMut(_)>);
    window
        .add_event_listener_with_callback("mouseup", cb.as_ref().unchecked_ref())
        .unwrap();
    cb.forget();

    let s = state.clone();
    let cb = Closure::wrap(Box::new(move |e: web_sys::WheelEvent| {
        let mut st = s.borrow_mut();
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
        let w = web_sys::window().unwrap();
        let dpr = w.device_pixel_ratio();
        let css_w = w.inner_width().unwrap().as_f64().unwrap() as u32;
        let css_h = w.inner_height().unwrap().as_f64().unwrap() as u32;
        let px_w = (css_w as f64 * dpr) as u32;
        let px_h = (css_h as f64 * dpr) as u32;

        st.canvas.set_width(px_w);
        st.canvas.set_height(px_h);
        st.canvas
            .style()
            .set_property("width", &format!("{css_w}px"))
            .unwrap();
        st.canvas
            .style()
            .set_property("height", &format!("{}px", css_h.saturating_sub(40)))
            .unwrap();
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

// ── Headless bench worker for interleaved A/B mode ──────────────────────────

/// Headless worker entry point for interleaved A/B benchmarking.
///
/// Instead of building the full UI and animation loop, this registers a
/// `message` event listener and responds to commands from a parent orchestrator
/// page via `postMessage`.
#[wasm_bindgen]
pub fn bench_worker_init() {
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Warn);

    let window = web_sys::window().unwrap();

    let cb = Closure::wrap(Box::new(move |event: web_sys::MessageEvent| {
        let data = event.data();
        let Some(obj) = data.dyn_ref::<js_sys::Object>() else {
            return;
        };
        let msg_type = js_sys::Reflect::get(obj, &"type".into())
            .ok()
            .and_then(|v| v.as_string());

        match msg_type.as_deref() {
            Some("get_defs") => {
                let defs = bench_defs();
                let arr = js_sys::Array::new();
                for (i, d) in defs.iter().enumerate() {
                    let entry = js_sys::Object::new();
                    js_sys::Reflect::set(&entry, &"idx".into(), &(i as u32).into()).unwrap();
                    js_sys::Reflect::set(&entry, &"name".into(), &d.name.into()).unwrap();
                    js_sys::Reflect::set(&entry, &"category".into(), &d.category.into()).unwrap();
                    js_sys::Reflect::set(&entry, &"description".into(), &d.description.into())
                        .unwrap();
                    arr.push(&entry);
                }
                let reply = js_sys::Object::new();
                js_sys::Reflect::set(&reply, &"type".into(), &"defs".into()).unwrap();
                js_sys::Reflect::set(&reply, &"defs".into(), &arr).unwrap();
                let _ = web_sys::window()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .unwrap()
                    .post_message(&reply, "*");
            }
            Some("run_bench") => {
                let get_f64 = |key: &str| {
                    js_sys::Reflect::get(obj, &key.into())
                        .ok()
                        .and_then(|v| v.as_f64())
                };
                let idx = get_f64("idx").unwrap_or(0.0) as usize;
                let preset = get_f64("preset").unwrap_or(10.0) as u32;
                let warmup_ms = get_f64("warmup_ms").unwrap_or(250.0);
                let run_ms = get_f64("run_ms").unwrap_or(1000.0);
                let width = get_f64("width").unwrap_or(800.0) as u32;
                let height = get_f64("height").unwrap_or(600.0) as u32;

                let reply = js_sys::Object::new();
                js_sys::Reflect::set(&reply, &"type".into(), &"bench_result".into()).unwrap();
                js_sys::Reflect::set(&reply, &"idx".into(), &(idx as u32).into()).unwrap();

                if let Some(result) =
                    harness::run_single_bench(idx, preset, warmup_ms, run_ms, width, height)
                {
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
                } else {
                    js_sys::Reflect::set(&reply, &"error".into(), &"invalid bench index".into())
                        .unwrap();
                }

                let _ = web_sys::window()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .unwrap()
                    .post_message(&reply, "*");
            }
            _ => {}
        }
    }) as Box<dyn FnMut(_)>);
    window
        .add_event_listener_with_callback("message", cb.as_ref().unchecked_ref())
        .unwrap();
    cb.forget();

    // Signal readiness to the parent orchestrator.
    let ready = js_sys::Object::new();
    js_sys::Reflect::set(&ready, &"type".into(), &"ready".into()).unwrap();
    let _ = window.parent().unwrap().unwrap().post_message(&ready, "*");
}
