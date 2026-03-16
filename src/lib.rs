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
mod fps;
pub(crate) mod harness;
pub(crate) mod rng;
pub mod scenes;
pub(crate) mod storage;
pub mod ui;

use std::cell::RefCell;
use std::rc::Rc;

use backend::{Backend, DrawContext};
use fps::FpsTracker;
use harness::{BenchDef, BenchHarness, HarnessEvent, bench_defs};
use scenes::BenchScene;
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
    scene: DrawContext,
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
            self.backend = Backend::new(&self.canvas);
            self.scene = backend::new_draw_context(self.width, self.height);
            self.scenes = scenes::all_scenes();
            self.reset_view();
            let params = self.scenes[self.current_scene].params();
            self.ui.rebuild_params(&params);
            self.ui.mark_dirty();
        }

        let params = self.ui.read_params();
        let idx = self.current_scene;
        for &(name, value) in &params {
            self.scenes[idx].set_param(name, value);
        }

        let perf = web_sys::window().unwrap().performance().unwrap();
        let t0 = perf.now();

        self.scene.reset();
        let (w, h) = (self.width, self.height);
        let view = Affine::translate((self.pan_x, self.pan_y)) * Affine::scale(self.zoom);
        self.scenes[idx].render(&mut self.scene, &mut self.backend, w, h, now, view);

        let encode_ms = perf.now() - t0;

        self.backend.render(&mut self.scene);
        self.backend.sync();

        let total_ms = perf.now() - t0;
        let (fps, frame_time) = self.fps_tracker.frame(now);
        self.ui.update_timing(fps, frame_time, encode_ms, total_ms);
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

#[cfg(not(feature = "cpu"))]
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

    let canvas: HtmlCanvasElement = document
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.set_width(px_w);
    canvas.set_height(px_h);
    let cs = canvas.style();
    cs.set_property("position", "fixed").unwrap();
    cs.set_property("top", "40px").unwrap(); // below top bar
    cs.set_property("left", "0").unwrap();
    cs.set_property("z-index", "0").unwrap();
    cs.set_property("width", &format!("{css_w}px")).unwrap();
    cs.set_property("height", &format!("{}px", css_h.saturating_sub(40)))
        .unwrap();
    document.body().unwrap().append_child(&canvas).unwrap();

    let bench_scenes = scenes::all_scenes();
    let defs = bench_defs();

    let saved_state = storage::load_ui_state();
    let initial_mode = match saved_state.mode.as_deref() {
        Some("interactive") => AppMode::Interactive,
        _ => AppMode::Benchmark,
    };
    let initial_scene = saved_state
        .scene
        .filter(|&i| i < bench_scenes.len())
        .unwrap_or(0);

    let ui = Ui::build(&document, &bench_scenes, &defs, initial_scene, px_w, px_h);
    let backend = Backend::new(&canvas);
    let scene = backend::new_draw_context(px_w, px_h);
    let now = performance.now();

    // Canvas visibility depends on initial mode.
    let canvas_vis = if initial_mode == AppMode::Interactive {
        "visible"
    } else {
        "hidden"
    };
    canvas
        .style()
        .set_property("visibility", canvas_vis)
        .unwrap();

    let state = Rc::new(RefCell::new(AppState {
        scenes: bench_scenes,
        current_scene: initial_scene,
        scene,
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
        st.ui.apply_saved_params(&saved_state);
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
        let canvas_ref = state.borrow().canvas.clone();
        let cb = Closure::wrap(Box::new(move || {
            let mut st = s.borrow_mut();
            st.ui.set_mode(AppMode::Interactive);
            st.ui.flush_state();
            canvas_ref
                .style()
                .set_property("visibility", "visible")
                .unwrap();
        }) as Box<dyn FnMut()>);
        itab.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();

        let s = state.clone();
        let canvas_ref = state.borrow().canvas.clone();
        let cb = Closure::wrap(Box::new(move || {
            let mut st = s.borrow_mut();
            st.ui.set_mode(AppMode::Benchmark);
            st.ui.flush_state();
            canvas_ref
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
                st.scene = backend::new_draw_context(vp_w, vp_h);
                st.backend.resize(vp_w, vp_h);
            }
            st.harness.warmup_ms = st.ui.warmup_ms();
            st.harness.run_ms = st.ui.run_ms();
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
    let dpr = window.device_pixel_ratio();
    let cb = Closure::wrap(Box::new(move |e: web_sys::MouseEvent| {
        let mut st = s.borrow_mut();
        if !st.dragging {
            return;
        }
        let x = e.client_x() as f64;
        let y = e.client_y() as f64;
        st.pan_x += (x - st.drag_last_x) * dpr;
        st.pan_y += (y - st.drag_last_y) * dpr;
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
        let dpr = web_sys::window().unwrap().device_pixel_ratio();
        let cx = e.client_x() as f64 * dpr;
        let cy = (e.client_y() as f64 - 40.0) * dpr;

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
            let dpr = web_sys::window().unwrap().device_pixel_ratio();
            let touches = e.touches();

            if touches.length() == 1 && st.touch_count == 1 {
                // Single finger pan.
                let t = touches.get(0).unwrap();
                let x = t.client_x() as f64;
                let y = t.client_y() as f64;
                st.pan_x += (x - st.touch_last_x) * dpr;
                st.pan_y += (y - st.touch_last_y) * dpr;
                st.touch_last_x = x;
                st.touch_last_y = y;
                st.update_reset_btn();
            } else if touches.length() >= 2 {
                // Pinch zoom + two-finger pan.
                let new_dist = touch_distance(&touches);
                let (mx, my) = touch_midpoint(&touches);

                if st.pinch_dist > 0.0 {
                    let factor = new_dist / st.pinch_dist;
                    let cx = mx * dpr;
                    let cy = (my - 40.0) * dpr;
                    st.zoom_at(cx, cy, factor);
                }
                // Pan by midpoint delta.
                st.pan_x += (mx - st.touch_last_x) * dpr;
                st.pan_y += (my - st.touch_last_y) * dpr;

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
        s.borrow_mut().tick(now);
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
        st.scene = backend::new_draw_context(px_w, px_h);
        st.backend.resize(px_w, px_h);
        st.ui.update_viewport(px_w, px_h);
    }) as Box<dyn FnMut(_)>);
    web_sys::window()
        .unwrap()
        .add_event_listener_with_callback("resize", cb.as_ref().unchecked_ref())
        .unwrap();
    cb.forget();
}
