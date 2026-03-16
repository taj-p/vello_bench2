//! Benchmark harness with warmup calibration and vsync-independent timing.

#![allow(
    clippy::cast_possible_truncation,
    reason = "truncation has no appreciable impact in this benchmark"
)]

use crate::backend::Backend;
use crate::scenes::{self, BenchScene};
use vello_common::kurbo::Affine;
use web_sys::HtmlCanvasElement;

/// A predefined benchmark with fixed parameters.
#[derive(Debug, Clone)]
pub(crate) struct BenchDef {
    /// Display name.
    pub(crate) name: &'static str,
    /// Short description of what this benchmark tests.
    pub(crate) description: &'static str,
    /// Category for grouping in the UI.
    pub(crate) category: &'static str,
    /// Which scene index to use.
    pub(crate) scene_idx: usize,
    /// Parameter overrides (speed is always forced to 0 on top of these).
    pub(crate) params: &'static [(&'static str, f64)],
}

/// Result of a single benchmark run.
#[derive(Debug, Clone)]
pub(crate) struct BenchResult {
    /// Benchmark name.
    pub(crate) name: &'static str,
    /// Average time per frame in milliseconds.
    pub(crate) ms_per_frame: f64,
    /// Number of iterations in the run phase.
    pub(crate) iterations: usize,
    /// Total wall-clock time of the run phase in milliseconds.
    #[allow(dead_code, reason = "useful for detailed output")]
    pub(crate) total_ms: f64,
}

/// Events emitted by the harness after each tick.
#[derive(Debug)]
pub(crate) enum HarnessEvent {
    /// The first warmup frame was just rendered — caller should capture a screenshot.
    ScreenshotReady,
    /// A single benchmark finished.
    BenchDone(BenchResult),
    /// All benchmarks finished.
    AllDone,
}

/// Current phase.
#[derive(Debug)]
enum Phase {
    Idle,
    PendingWarmup(usize),
    PendingRun { idx: usize, target_iters: usize },
    Complete,
}

/// Orchestrates running benchmarks.
///
/// The harness creates its own fresh context and bench scene instances
/// for each benchmark to ensure complete isolation from interactive mode
/// and between test cases.
pub(crate) struct BenchHarness {
    phase: Phase,
    pub(crate) warmup_ms: f64,
    pub(crate) run_ms: f64,
    pub(crate) results: Vec<BenchResult>,
    run_order: Vec<usize>,
    run_pos: usize,
    bench_scenes: Option<Vec<Box<dyn BenchScene>>>,
    bench_backend: Option<Backend>,
}

impl std::fmt::Debug for BenchHarness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BenchHarness")
            .field("phase", &self.phase)
            .field("warmup_ms", &self.warmup_ms)
            .field("run_ms", &self.run_ms)
            .finish_non_exhaustive()
    }
}

impl BenchHarness {
    pub(crate) fn new() -> Self {
        Self {
            phase: Phase::Idle,
            warmup_ms: 250.0,
            run_ms: 1000.0,
            results: Vec::new(),
            run_order: Vec::new(),
            run_pos: 0,
            bench_scenes: None,
            bench_backend: None,
        }
    }

    /// Start with a specific set of def indices to run (in order).
    pub(crate) fn start(
        &mut self,
        selected: Vec<usize>,
        width: u32,
        height: u32,
        canvas: &HtmlCanvasElement,
    ) {
        self.results.clear();
        self.run_order = selected;
        self.run_pos = 0;
        self.bench_scenes = Some(scenes::all_scenes());
        self.bench_backend = Some(Backend::new(canvas, width, height));
        if self.run_order.is_empty() {
            self.phase = Phase::Complete;
        } else {
            self.phase = Phase::PendingWarmup(self.run_order[0]);
        }
    }

    pub(crate) fn is_running(&self) -> bool {
        !matches!(self.phase, Phase::Idle | Phase::Complete)
    }

    pub(crate) fn current_bench_idx(&self) -> Option<usize> {
        match &self.phase {
            Phase::PendingWarmup(i) | Phase::PendingRun { idx: i, .. } => Some(*i),
            _ => None,
        }
    }

    /// Drive one step. Returns events for the caller to act on.
    pub(crate) fn tick(&mut self, defs: &[BenchDef], width: u32, height: u32) -> Vec<HarnessEvent> {
        let mut events = Vec::new();

        match self.phase {
            Phase::Idle | Phase::Complete => {}
            Phase::PendingWarmup(idx) => {
                let def = &defs[idx];
                let bench_scenes = self.bench_scenes.as_mut().unwrap();
                let scene = &mut *bench_scenes[def.scene_idx];
                apply_params(scene, def.params);

                let be = self.bench_backend.as_mut().unwrap();
                be.reset_with_size(width, height);
                let perf = web_sys::window().unwrap().performance().unwrap();

                let now = perf.now();
                render_one(scene, be, width, height, now);
                be.sync();
                events.push(HarnessEvent::ScreenshotReady);

                let start = perf.now();
                let mut count = 0_usize;
                loop {
                    let t = perf.now();
                    render_one(scene, be, width, height, t);
                    be.sync();
                    count += 1;
                    if perf.now() - start >= self.warmup_ms {
                        break;
                    }
                }
                let elapsed = perf.now() - start;
                let rate = count as f64 / elapsed;
                let target = (rate * self.run_ms).max(1.0) as usize;

                self.phase = Phase::PendingRun {
                    idx,
                    target_iters: target,
                };
            }
            Phase::PendingRun { idx, target_iters } => {
                let def = &defs[idx];
                let bench_scenes = self.bench_scenes.as_mut().unwrap();
                let scene = &mut *bench_scenes[def.scene_idx];
                let be = self.bench_backend.as_mut().unwrap();

                let perf = web_sys::window().unwrap().performance().unwrap();
                let start = perf.now();
                for _ in 0..target_iters {
                    let t = perf.now();
                    render_one(scene, be, width, height, t);
                    be.sync();
                }
                let total_ms = perf.now() - start;

                let result = BenchResult {
                    name: def.name,
                    ms_per_frame: total_ms / target_iters as f64,
                    iterations: target_iters,
                    total_ms,
                };
                self.results.push(result.clone());
                events.push(HarnessEvent::BenchDone(result));

                self.run_pos += 1;
                if self.run_pos < self.run_order.len() {
                    self.phase = Phase::PendingWarmup(self.run_order[self.run_pos]);
                } else {
                    self.phase = Phase::Complete;
                    self.bench_scenes = None;
                    self.bench_backend = None;
                    events.push(HarnessEvent::AllDone);
                }
            }
        }

        events
    }
}

fn apply_params(scene: &mut dyn BenchScene, params: &[(&str, f64)]) {
    for &(name, value) in params {
        scene.set_param(name, value);
    }
    // Always force speed=0 for deterministic benchmarks.
    scene.set_param("speed", 0.0);
}

fn render_one(
    bench_scene: &mut dyn BenchScene,
    backend: &mut Backend,
    width: u32,
    height: u32,
    time: f64,
) {
    backend.reset();
    bench_scene.render(backend, width, height, time, Affine::IDENTITY);
    backend.render_offscreen();
    backend.blit();
}

/// All predefined benchmarks.
pub(crate) fn bench_defs() -> Vec<BenchDef> {
    vec![
        BenchDef {
            name: "200k Rect - 5×5 - Solid",
            description: "rendering small rectangles",
            category: "Rectangles",
            scene_idx: 0,
            params: &[
                ("num_rects", 200_000.0),
                ("rect_size", 5.0),
                ("paint_mode", 0.0),
                ("rotated", 0.0),
            ],
        },
        BenchDef {
            name: "50k Rect - 50×50 - Solid",
            description: "rendering medium-sized rectangles",
            category: "Rectangles",
            scene_idx: 0,
            params: &[
                ("num_rects", 50_000.0),
                ("rect_size", 50.0),
                ("paint_mode", 0.0),
                ("rotated", 0.0),
            ],
        },
        BenchDef {
            name: "10k Rect - 200×200 - Solid",
            description: "rendering large rectangles",
            category: "Rectangles",
            scene_idx: 0,
            params: &[
                ("num_rects", 10_000.0),
                ("rect_size", 200.0),
                ("paint_mode", 0.0),
                ("rotated", 0.0),
            ],
        },
        BenchDef {
            name: "10k Rect - 200×200 - Image - Nearest",
            description: "rendering transparent images with NN sampling",
            category: "Images",
            scene_idx: 0,
            params: &[
                ("num_rects", 10_000.0),
                ("rect_size", 200.0),
                ("paint_mode", 2.0),
                ("rotated", 0.0),
                ("image_filter", 0.0),
                ("image_opaque", 0.0),
            ],
        },
        BenchDef {
            name: "10k Rect - 200×200 - Image - Bilinear",
            description: "rendering transparent images with bilinear sampling",
            category: "Images",
            scene_idx: 0,
            params: &[
                ("num_rects", 10_000.0),
                ("rect_size", 200.0),
                ("paint_mode", 2.0),
                ("rotated", 0.0),
                ("image_filter", 1.0),
                ("image_opaque", 0.0),
            ],
        },
        BenchDef {
            name: "10k Rect - 200×200 - Opaque Image - Nearest",
            description: "rendering opaque images with NN sampling",
            category: "Images",
            scene_idx: 0,
            params: &[
                ("num_rects", 10_000.0),
                ("rect_size", 200.0),
                ("paint_mode", 2.0),
                ("rotated", 0.0),
                ("image_filter", 0.0),
                ("image_opaque", 1.0),
            ],
        },
        BenchDef {
            name: "10k Rect - 200×200 - Opaque Image - Bilinear",
            description: "rendering opaque images with bilinear sampling",
            category: "Images",
            scene_idx: 0,
            params: &[
                ("num_rects", 10_000.0),
                ("rect_size", 200.0),
                ("paint_mode", 2.0),
                ("rotated", 0.0),
                ("image_filter", 1.0),
                ("image_opaque", 1.0),
            ],
        },
        BenchDef {
            name: "10k Rect - 200×200 - Opaque Image (draw_image) - Nearest",
            description: "rendering images via draw_image API (GPU fast path on hybrid)",
            category: "Images",
            scene_idx: 0,
            params: &[
                ("num_rects", 10_000.0),
                ("rect_size", 200.0),
                ("paint_mode", 2.0),
                ("rotated", 0.0),
                ("image_filter", 0.0),
                ("image_opaque", 1.0),
                ("use_draw_image", 1.0),
            ],
        },
        BenchDef {
            name: "10k Rect - 200×200 - Opaque Image (draw_image) - Bilinear",
            description: "rendering images via draw_image API with bilinear (GPU fast path on hybrid)",
            category: "Images",
            scene_idx: 0,
            params: &[
                ("num_rects", 10_000.0),
                ("rect_size", 200.0),
                ("paint_mode", 2.0),
                ("rotated", 0.0),
                ("image_filter", 1.0),
                ("image_opaque", 1.0),
                ("use_draw_image", 1.0),
            ],
        },
        BenchDef {
            name: "700 Stroked Lines - 3px",
            description: "rendering lines with small stroke width",
            category: "Strokes",
            scene_idx: 1,
            params: &[
                ("num_strokes", 700.0),
                ("curve_type", 0.0),
                ("stroke_width", 3.0),
            ],
        },
        BenchDef {
            name: "700 Stroked Lines - 20px",
            description: "rendering lines with large stroke width",
            category: "Strokes",
            scene_idx: 1,
            params: &[
                ("num_strokes", 700.0),
                ("curve_type", 0.0),
                ("stroke_width", 20.0),
            ],
        },
        BenchDef {
            name: "700 Stroked Quads - 3px",
            description: "rendering quads with small stroke width",
            category: "Strokes",
            scene_idx: 1,
            params: &[
                ("num_strokes", 700.0),
                ("curve_type", 1.0),
                ("stroke_width", 3.0),
            ],
        },
        BenchDef {
            name: "700 Stroked Quads - 20px",
            description: "rendering quads with large stroke width",
            category: "Strokes",
            scene_idx: 1,
            params: &[
                ("num_strokes", 700.0),
                ("curve_type", 1.0),
                ("stroke_width", 20.0),
            ],
        },
        BenchDef {
            name: "700 Stroked Cubics - 3px",
            description: "rendering cubics with small stroke width",
            category: "Strokes",
            scene_idx: 1,
            params: &[
                ("num_strokes", 700.0),
                ("curve_type", 2.0),
                ("stroke_width", 3.0),
            ],
        },
        BenchDef {
            name: "700 Stroked Cubics - 20px",
            description: "rendering cubics with large stroke width",
            category: "Strokes",
            scene_idx: 1,
            params: &[
                ("num_strokes", 700.0),
                ("curve_type", 2.0),
                ("stroke_width", 20.0),
            ],
        },
        BenchDef {
            name: "Polyline - 2500 vertices",
            description: "rendering paths bottlenecked by tiling and strip rendering",
            category: "Fills",
            scene_idx: 2,
            params: &[("num_vertices", 2000.0)],
        },
        BenchDef {
            name: "Ghostscript Tiger",
            description: "rendering simple vector graphics",
            category: "Vector Graphics",
            scene_idx: 3,
            params: &[("svg_asset", 0.0)],
        },
        BenchDef {
            name: "Coat of Arms",
            description: "rendering simple vector graphics",
            category: "Vector Graphics",
            scene_idx: 3,
            params: &[("svg_asset", 1.0)],
        },
        BenchDef {
            name: "Heraldry",
            description: "rendering simple vector graphics",
            category: "Vector Graphics",
            scene_idx: 3,
            params: &[("svg_asset", 2.0)],
        },
        BenchDef {
            name: "400 Rect - 400px - Global `clip_path`",
            description: "rendering many paths with a single clip path using `push_clip_path`",
            category: "Clip Paths",
            scene_idx: 4,
            params: &[
                ("num_rects", 400.0),
                ("rect_size", 400.0),
                ("clip_mode", 1.0),
                ("clip_method", 0.0),
            ],
        },
        BenchDef {
            name: "400 Rect - 400px - Global `clip_layer`",
            description: "rendering many paths with a single clip path using `push_clip_layer`",
            category: "Clip Paths",
            scene_idx: 4,
            params: &[
                ("num_rects", 400.0),
                ("rect_size", 400.0),
                ("clip_mode", 1.0),
                ("clip_method", 1.0),
            ],
        },
        BenchDef {
            name: "200 Rect - 200px - Per-shape `clip_path`",
            description: "rendering many paths with many clip paths using `push_clip_path`",
            category: "Clip Paths",
            scene_idx: 4,
            params: &[
                ("num_rects", 200.0),
                ("rect_size", 200.0),
                ("clip_mode", 2.0),
                ("clip_method", 0.0),
            ],
        },
        BenchDef {
            name: "200 Rect - 200px - Per-shape `clip_layer`",
            description: "rendering many paths with many clip paths using `push_clip_layer`",
            category: "Clip Paths",
            scene_idx: 4,
            params: &[
                ("num_rects", 200.0),
                ("rect_size", 200.0),
                ("clip_mode", 2.0),
                ("clip_method", 1.0),
            ],
        },
        BenchDef {
            name: "500 Text - 8px",
            description: "rendering small text",
            category: "Text",
            scene_idx: 5,
            params: &[("num_runs", 500.0), ("font_size", 8.0)],
        },
        BenchDef {
            name: "500 Text - 24px",
            description: "rendering medium-sized text",
            category: "Text",
            scene_idx: 5,
            params: &[("num_runs", 500.0), ("font_size", 24.0)],
        },
        BenchDef {
            name: "500 Text - 60px",
            description: "rendering large text",
            category: "Text",
            scene_idx: 5,
            params: &[("num_runs", 500.0), ("font_size", 60.0)],
        },
    ]
}
