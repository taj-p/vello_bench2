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
    /// Optional count parameter scaled by the benchmark preset.
    pub(crate) scale: Option<BenchScale>,
    /// Parameter overrides (speed is always forced to 0 on top of these).
    pub(crate) params: &'static [(&'static str, f64)],
}

/// Scaling metadata for a benchmark count parameter.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BenchScale {
    pub(crate) param: &'static str,
    pub(crate) calibrated_value: usize,
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
    pub(crate) preset: u32,
    pub(crate) results: Vec<BenchResult>,
    run_order: Vec<usize>,
    run_pos: usize,
    bench_scene: Option<Box<dyn BenchScene>>,
    bench_canvas: Option<HtmlCanvasElement>,
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
            preset: 10,
            results: Vec::new(),
            run_order: Vec::new(),
            run_pos: 0,
            bench_scene: None,
            bench_canvas: None,
            bench_backend: None,
        }
    }

    /// Start with a specific set of def indices to run (in order).
    pub(crate) fn start(
        &mut self,
        selected: Vec<usize>,
        _width: u32,
        _height: u32,
        canvas: &HtmlCanvasElement,
    ) {
        self.results.clear();
        self.run_order = selected;
        self.run_pos = 0;
        self.bench_scene = None;
        self.bench_canvas = Some(canvas.clone());
        self.bench_backend = None;
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
                let mut bench_scenes = scenes::all_scenes();
                let scene = bench_scenes.swap_remove(def.scene_idx);
                self.bench_scene = Some(scene);
                let scene = self.bench_scene.as_mut().unwrap().as_mut();
                apply_params(scene, def.params, def.scale, self.preset);

                let canvas = self.bench_canvas.as_ref().unwrap();
                self.bench_backend = Some(Backend::new(canvas, width, height));
                let be = self.bench_backend.as_mut().unwrap();
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
                let scene = self.bench_scene.as_mut().unwrap().as_mut();
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
                    self.bench_scene = None;
                    self.bench_backend = None;
                    self.phase = Phase::PendingWarmup(self.run_order[self.run_pos]);
                } else {
                    self.phase = Phase::Complete;
                    self.bench_scene = None;
                    self.bench_canvas = None;
                    self.bench_backend = None;
                    events.push(HarnessEvent::AllDone);
                }
            }
        }

        events
    }
}

fn apply_params(
    scene: &mut dyn BenchScene,
    params: &[(&str, f64)],
    scale: Option<BenchScale>,
    preset: u32,
) {
    for &(name, value) in params {
        scene.set_param(name, value);
    }
    if let Some(scale) = scale {
        scene.set_param(
            scale.param,
            scaled_count(scale.calibrated_value, preset) as f64,
        );
    }
    // Always force speed=0 for deterministic benchmarks.
    scene.set_param("speed", 0.0);
}

pub(crate) fn scaled_count(calibrated_value: usize, preset: u32) -> usize {
    if preset <= 1 {
        return 1;
    }
    let max_value = calibrated_value.saturating_mul(4).max(1);
    let exponent = (preset - 1) as f64 / 19.0;
    (max_value as f64).powf(exponent).ceil().max(1.0) as usize
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
            name: "Rect - 5×5 - Solid",
            description: "rendering small rectangles",
            category: "Rectangles",
            scene_idx: 0,
            scale: Some(BenchScale {
                param: "num_rects",
                calibrated_value: 600_000,
            }),
            params: &[
                ("num_rects", 600_000.0),
                ("rect_size", 5.0),
                ("paint_mode", 0.0),
                ("rotated", 0.0),
            ],
        },
        BenchDef {
            name: "Rect - 50×50 - Solid",
            description: "rendering medium-sized rectangles",
            category: "Rectangles",
            scene_idx: 0,
            scale: Some(BenchScale {
                param: "num_rects",
                calibrated_value: 380_000,
            }),
            params: &[
                ("num_rects", 380_000.0),
                ("rect_size", 50.0),
                ("paint_mode", 0.0),
                ("rotated", 0.0),
            ],
        },
        BenchDef {
            name: "Rect - 200×200 - Solid",
            description: "rendering large rectangles",
            category: "Rectangles",
            scene_idx: 0,
            scale: Some(BenchScale {
                param: "num_rects",
                calibrated_value: 53_000,
            }),
            params: &[
                ("num_rects", 53_000.0),
                ("rect_size", 200.0),
                ("paint_mode", 0.0),
                ("rotated", 0.0),
            ],
        },
        BenchDef {
            name: "Rect - 200×200 - Image - Nearest",
            description: "rendering transparent images with NN sampling",
            category: "Images",
            scene_idx: 0,
            scale: Some(BenchScale {
                param: "num_rects",
                calibrated_value: 30_000,
            }),
            params: &[
                ("num_rects", 30_000.0),
                ("rect_size", 200.0),
                ("paint_mode", 2.0),
                ("rotated", 0.0),
                ("image_filter", 0.0),
                ("image_opaque", 0.0),
            ],
        },
        BenchDef {
            name: "Rect - 200×200 - Image - Bilinear",
            description: "rendering transparent images with bilinear sampling",
            category: "Images",
            scene_idx: 0,
            scale: Some(BenchScale {
                param: "num_rects",
                calibrated_value: 24_000,
            }),
            params: &[
                ("num_rects", 24_000.0),
                ("rect_size", 200.0),
                ("paint_mode", 2.0),
                ("rotated", 0.0),
                ("image_filter", 1.0),
                ("image_opaque", 0.0),
            ],
        },
        BenchDef {
            name: "Rect - 200×200 - Opaque Image - Nearest",
            description: "rendering opaque images with NN sampling",
            category: "Images",
            scene_idx: 0,
            scale: Some(BenchScale {
                param: "num_rects",
                calibrated_value: 31_000,
            }),
            params: &[
                ("num_rects", 31_000.0),
                ("rect_size", 200.0),
                ("paint_mode", 2.0),
                ("rotated", 0.0),
                ("image_filter", 0.0),
                ("image_opaque", 1.0),
            ],
        },
        BenchDef {
            name: "Rect - 200×200 - Opaque Image - Bilinear",
            description: "rendering opaque images with bilinear sampling",
            category: "Images",
            scene_idx: 0,
            scale: Some(BenchScale {
                param: "num_rects",
                calibrated_value: 24_000,
            }),
            params: &[
                ("num_rects", 24_000.0),
                ("rect_size", 200.0),
                ("paint_mode", 2.0),
                ("rotated", 0.0),
                ("image_filter", 1.0),
                ("image_opaque", 1.0),
            ],
        },
        BenchDef {
            name: "Rect - 200×200 - Opaque Image (draw_image) - Nearest",
            description: "rendering images via draw_image API (GPU fast path on hybrid)",
            category: "Images",
            scene_idx: 0,
            scale: Some(BenchScale {
                param: "num_rects",
                calibrated_value: 35_000,
            }),
            params: &[
                ("num_rects", 35_000.0),
                ("rect_size", 200.0),
                ("paint_mode", 2.0),
                ("rotated", 0.0),
                ("image_filter", 0.0),
                ("image_opaque", 1.0),
                ("use_draw_image", 1.0),
            ],
        },
        BenchDef {
            name: "Rect - 200×200 - Opaque Image (draw_image) - Bilinear",
            description: "rendering images via draw_image API with bilinear (GPU fast path on hybrid)",
            category: "Images",
            scene_idx: 0,
            scale: Some(BenchScale {
                param: "num_rects",
                calibrated_value: 34_000,
            }),
            params: &[
                ("num_rects", 34_000.0),
                ("rect_size", 200.0),
                ("paint_mode", 2.0),
                ("rotated", 0.0),
                ("image_filter", 1.0),
                ("image_opaque", 1.0),
                ("use_draw_image", 1.0),
            ],
        },
        BenchDef {
            name: "Stroked Lines - 3px",
            description: "rendering lines with small stroke width",
            category: "Strokes",
            scene_idx: 1,
            scale: Some(BenchScale {
                param: "num_strokes",
                calibrated_value: 18_500,
            }),
            params: &[
                ("num_strokes", 18_500.0),
                ("curve_type", 0.0),
                ("stroke_width", 3.0),
            ],
        },
        BenchDef {
            name: "Stroked Lines - 20px",
            description: "rendering lines with large stroke width",
            category: "Strokes",
            scene_idx: 1,
            scale: Some(BenchScale {
                param: "num_strokes",
                calibrated_value: 13_200,
            }),
            params: &[
                ("num_strokes", 13_200.0),
                ("curve_type", 0.0),
                ("stroke_width", 20.0),
            ],
        },
        BenchDef {
            name: "Stroked Quads - 3px",
            description: "rendering quads with small stroke width",
            category: "Strokes",
            scene_idx: 1,
            scale: Some(BenchScale {
                param: "num_strokes",
                calibrated_value: 6_900,
            }),
            params: &[
                ("num_strokes", 6_900.0),
                ("curve_type", 1.0),
                ("stroke_width", 3.0),
            ],
        },
        BenchDef {
            name: "Stroked Quads - 20px",
            description: "rendering quads with large stroke width",
            category: "Strokes",
            scene_idx: 1,
            scale: Some(BenchScale {
                param: "num_strokes",
                calibrated_value: 5_100,
            }),
            params: &[
                ("num_strokes", 5_100.0),
                ("curve_type", 1.0),
                ("stroke_width", 20.0),
            ],
        },
        BenchDef {
            name: "Stroked Cubics - 3px",
            description: "rendering cubics with small stroke width",
            category: "Strokes",
            scene_idx: 1,
            scale: Some(BenchScale {
                param: "num_strokes",
                calibrated_value: 5_000,
            }),
            params: &[
                ("num_strokes", 5_000.0),
                ("curve_type", 2.0),
                ("stroke_width", 3.0),
            ],
        },
        BenchDef {
            name: "Stroked Cubics - 20px",
            description: "rendering cubics with large stroke width",
            category: "Strokes",
            scene_idx: 1,
            scale: Some(BenchScale {
                param: "num_strokes",
                calibrated_value: 3_500,
            }),
            params: &[
                ("num_strokes", 3_500.0),
                ("curve_type", 2.0),
                ("stroke_width", 20.0),
            ],
        },
        BenchDef {
            name: "Polyline",
            description: "rendering paths bottlenecked by tiling and strip rendering",
            category: "Fills",
            scene_idx: 2,
            scale: Some(BenchScale {
                param: "num_vertices",
                calibrated_value: 2_200,
            }),
            params: &[("num_vertices", 2200.0)],
        },
        BenchDef {
            name: "Ghostscript Tiger",
            description: "rendering simple vector graphics",
            category: "Vector Graphics",
            scene_idx: 3,
            scale: None,
            params: &[("svg_asset", 0.0)],
        },
        BenchDef {
            name: "Coat of Arms",
            description: "rendering simple vector graphics",
            category: "Vector Graphics",
            scene_idx: 3,
            scale: None,
            params: &[("svg_asset", 1.0)],
        },
        BenchDef {
            name: "Heraldry",
            description: "rendering simple vector graphics",
            category: "Vector Graphics",
            scene_idx: 3,
            scale: None,
            params: &[("svg_asset", 2.0)],
        },
        BenchDef {
            name: "Rect - 400px - Global `clip_path`",
            description: "rendering many paths with a single clip path using `push_clip_path`",
            category: "Clip Paths",
            scene_idx: 4,
            scale: Some(BenchScale {
                param: "num_rects",
                calibrated_value: 2_100,
            }),
            params: &[
                ("num_rects", 2_100.0),
                ("rect_size", 400.0),
                ("clip_mode", 1.0),
                ("clip_method", 0.0),
            ],
        },
        BenchDef {
            name: "Rect - 400px - Global `clip_layer`",
            description: "rendering many paths with a single clip path using `push_clip_layer`",
            category: "Clip Paths",
            scene_idx: 4,
            scale: Some(BenchScale {
                param: "num_rects",
                calibrated_value: 2_100,
            }),
            params: &[
                ("num_rects", 2_100.0),
                ("rect_size", 400.0),
                ("clip_mode", 1.0),
                ("clip_method", 1.0),
            ],
        },
        BenchDef {
            name: "Rect - 200px - Per-shape `clip_path`",
            description: "rendering many paths with many clip paths using `push_clip_path`",
            category: "Clip Paths",
            scene_idx: 4,
            scale: Some(BenchScale {
                param: "num_rects",
                calibrated_value: 930,
            }),
            params: &[
                ("num_rects", 930.0),
                ("rect_size", 200.0),
                ("clip_mode", 2.0),
                ("clip_method", 0.0),
            ],
        },
        BenchDef {
            name: "Rect - 200px - Per-shape `clip_layer`",
            description: "rendering many paths with many clip paths using `push_clip_layer`",
            category: "Clip Paths",
            scene_idx: 4,
            scale: Some(BenchScale {
                param: "num_rects",
                calibrated_value: 930,
            }),
            params: &[
                ("num_rects", 930.0),
                ("rect_size", 200.0),
                ("clip_mode", 2.0),
                ("clip_method", 1.0),
            ],
        },
        BenchDef {
            name: "Text - 8px",
            description: "rendering small text",
            category: "Text",
            scene_idx: 5,
            scale: Some(BenchScale {
                param: "num_runs",
                calibrated_value: 2_900,
            }),
            params: &[("num_runs", 2_900.0), ("font_size", 8.0)],
        },
        BenchDef {
            name: "Text - 24px",
            description: "rendering medium-sized text",
            category: "Text",
            scene_idx: 5,
            scale: Some(BenchScale {
                param: "num_runs",
                calibrated_value: 2_200,
            }),
            params: &[("num_runs", 2_200.0), ("font_size", 24.0)],
        },
        BenchDef {
            name: "Text - 60px",
            description: "rendering large text",
            category: "Text",
            scene_idx: 5,
            scale: Some(BenchScale {
                param: "num_runs",
                calibrated_value: 1_300,
            }),
            params: &[("num_runs", 1_300.0), ("font_size", 60.0)],
        },
    ]
}
