//! Benchmark harness with fixed-size frame averaging.

#![allow(
    clippy::cast_possible_truncation,
    reason = "truncation has no appreciable impact in this benchmark"
)]

use wasm_bindgen::JsCast;

use crate::backend::{Backend, current_backend_kind, new_backend};
use crate::resource_store::ResourceStore;
use crate::scenes::{BenchScene, ParamId, SceneId, new_scene};
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
    pub(crate) scene_id: SceneId,
    /// Optional count parameter scaled by the benchmark preset.
    pub(crate) scale: Option<BenchScale>,
    /// Parameter overrides (speed is always forced to 0 on top of these).
    pub(crate) params: &'static [(ParamId, f64)],
}

/// Scaling metadata for a benchmark count parameter.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BenchScale {
    pub(crate) param: ParamId,
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
    /// A single benchmark finished.
    BenchDone(BenchResult),
    /// All benchmarks finished.
    AllDone,
}

/// Current phase.
#[derive(Debug)]
enum Phase {
    Idle,
    PendingBench(usize),
    Running {
        idx: usize,
        last_now: f64,
        warmup_remaining: usize,
        total_ms: f64,
        samples: usize,
    },
    Complete,
}

const BENCH_WARMUP_SAMPLES: usize = 3;
const BENCH_MEASURED_SAMPLES: usize = 15;

/// Orchestrates running benchmarks.
///
/// The harness creates its own fresh context and bench scene instances
/// for each benchmark to ensure complete isolation from interactive mode
/// and between test cases.
pub(crate) struct BenchHarness {
    phase: Phase,
    pub(crate) preset: u32,
    pub(crate) results: Vec<BenchResult>,
    run_order: Vec<usize>,
    run_pos: usize,
    bench_scene: Option<Box<dyn BenchScene>>,
    bench_canvas: Option<HtmlCanvasElement>,
    bench_backend: Option<Box<dyn Backend>>,
    backend_kind: Option<crate::backend::BackendKind>,
    backend_width: u32,
    backend_height: u32,
    resources: ResourceStore,
}

impl std::fmt::Debug for BenchHarness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BenchHarness")
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

impl BenchHarness {
    pub(crate) fn new() -> Self {
        Self {
            phase: Phase::Idle,
            preset: 10,
            results: Vec::new(),
            run_order: Vec::new(),
            run_pos: 0,
            bench_scene: None,
            bench_canvas: None,
            bench_backend: None,
            backend_kind: None,
            backend_width: 0,
            backend_height: 0,
            resources: ResourceStore::new(),
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
        self.cleanup_current_bench();
        self.results.clear();
        self.run_order = selected;
        self.run_pos = 0;
        self.bench_canvas = Some(canvas.clone());
        if self.run_order.is_empty() {
            self.phase = Phase::Complete;
        } else {
            self.phase = Phase::PendingBench(self.run_order[0]);
        }
    }

    pub(crate) fn is_running(&self) -> bool {
        !matches!(self.phase, Phase::Idle | Phase::Complete)
    }

    pub(crate) fn current_bench_idx(&self) -> Option<usize> {
        match &self.phase {
            Phase::PendingBench(i) | Phase::Running { idx: i, .. } => Some(*i),
            _ => None,
        }
    }

    /// Drive one step. Returns events for the caller to act on.
    pub(crate) fn tick(
        &mut self,
        defs: &[BenchDef],
        width: u32,
        height: u32,
        now: f64,
    ) -> Vec<HarnessEvent> {
        let mut events = Vec::new();

        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Idle | Phase::Complete => {}
            Phase::PendingBench(idx) => {
                let def = &defs[idx];
                self.prepare_bench(def, width, height);
                let scene = self.bench_scene.as_mut().unwrap().as_mut();
                let be = self.bench_backend.as_mut().unwrap();
                render_one(scene, be.as_mut(), &mut self.resources, width, height, now);
                self.phase = Phase::Running {
                    idx,
                    last_now: now,
                    warmup_remaining: BENCH_WARMUP_SAMPLES,
                    total_ms: 0.0,
                    samples: 0,
                };
            }
            Phase::Running {
                idx,
                last_now,
                mut warmup_remaining,
                mut total_ms,
                mut samples,
            } => {
                let def = &defs[idx];
                let scene = self.bench_scene.as_mut().unwrap().as_mut();
                let be = self.bench_backend.as_mut().unwrap();
                render_one(scene, be.as_mut(), &mut self.resources, width, height, now);
                let dt = (now - last_now).max(0.0);
                if warmup_remaining > 0 {
                    warmup_remaining -= 1;
                } else {
                    total_ms += dt;
                    samples += 1;
                }

                if samples < BENCH_MEASURED_SAMPLES {
                    self.phase = Phase::Running {
                        idx,
                        last_now: now,
                        warmup_remaining,
                        total_ms,
                        samples,
                    };
                } else {
                    let result = BenchResult {
                        name: def.name,
                        ms_per_frame: total_ms / BENCH_MEASURED_SAMPLES as f64,
                        iterations: BENCH_MEASURED_SAMPLES,
                        total_ms,
                    };
                    self.results.push(result.clone());
                    events.push(HarnessEvent::BenchDone(result));

                    self.run_pos += 1;
                    if self.run_pos < self.run_order.len() {
                        self.phase = Phase::PendingBench(self.run_order[self.run_pos]);
                    } else {
                        self.phase = Phase::Complete;
                        self.cleanup_current_bench();
                        self.bench_canvas = None;
                        events.push(HarnessEvent::AllDone);
                    }
                }
            }
        }

        events
    }

    fn cleanup_current_bench(&mut self) {
        if let Some(backend) = self.bench_backend.as_mut() {
            self.resources.clear_all(backend.as_mut());
        }
        self.bench_scene = None;
    }

    fn prepare_bench(&mut self, def: &BenchDef, width: u32, height: u32) {
        // Note: We reuse the renderer whenever possible. This does have the advantage
        // that some state can leak across benchmarks (for example, if the alpha texture
        // grows very large in one frame then all subsequent benchmarks will also be affected
        // by that). The cleaner way would be to create a new renderer each benchmark,
        // but taht seems to crash my Samsung Tablet very easily (either because of
        // OOM or WebGL context loss), haven't investigated why yet.
        let kind = current_backend_kind();
        let needs_backend_rebuild = self.bench_backend.is_none()
            || self.backend_kind != Some(kind)
            || self.backend_width != width
            || self.backend_height != height;
        if needs_backend_rebuild {
            self.cleanup_current_bench();
            let canvas = self.bench_canvas.as_ref().unwrap();
            self.bench_backend = Some(new_backend(canvas, width, height, kind));
            self.backend_kind = Some(kind);
            self.backend_width = width;
            self.backend_height = height;
        }

        self.bench_scene = Some(new_scene(def.scene_id));
        if let Some(backend) = self.bench_backend.as_mut() {
            self.resources.clear_all(backend.as_mut());
        }
        let scene = self.bench_scene.as_mut().unwrap().as_mut();
        apply_params(scene, def.params, def.scale, self.preset);
    }
}

fn apply_params(
    scene: &mut dyn BenchScene,
    params: &[(ParamId, f64)],
    scale: Option<BenchScale>,
    preset: u32,
) {
    for &(param, value) in params {
        scene.set_param(param, value);
    }
    if let Some(scale) = scale {
        scene.set_param(
            scale.param,
            scaled_count(scale.calibrated_value, preset) as f64,
        );
    }
    // Always force speed=0 for deterministic benchmarks.
    scene.set_param(ParamId::Speed, 0.0);
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
    backend: &mut dyn Backend,
    resources: &mut ResourceStore,
    width: u32,
    height: u32,
    time: f64,
) {
    backend.reset();
    bench_scene.render(backend, resources, width, height, time, Affine::IDENTITY);
    backend.render_offscreen();
    backend.blit();
}

/// Run a single benchmark by index, creating a temporary canvas and backend.
/// Used by the headless worker mode for interleaved A/B testing.
pub fn run_single_bench(idx: usize, preset: u32, width: u32, height: u32) -> Option<BenchResult> {
    let defs = bench_defs();
    let def = defs.get(idx)?;

    let document = web_sys::window().unwrap().document().unwrap();
    let canvas: HtmlCanvasElement = document
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.set_width(width);
    canvas.set_height(height);
    document.body().unwrap().append_child(&canvas).unwrap();

    let mut scene = new_scene(def.scene_id);
    let scene = scene.as_mut();
    apply_params(scene, def.params, def.scale, preset);

    let mut be = new_backend(&canvas, width, height, current_backend_kind());
    let mut resources = ResourceStore::new();
    let perf = web_sys::window().unwrap().performance().unwrap();
    let mut last = perf.now();
    let mut total_ms = 0.0;
    for i in 0..(BENCH_WARMUP_SAMPLES + BENCH_MEASURED_SAMPLES) {
        let now = perf.now();
        render_one(scene, be.as_mut(), &mut resources, width, height, now);
        if i >= BENCH_WARMUP_SAMPLES {
            total_ms += (now - last).max(0.0);
        }
        last = now;
    }

    resources.clear_all(be.as_mut());

    // Clean up the temporary canvas.
    document.body().unwrap().remove_child(&canvas).unwrap();

    Some(BenchResult {
        name: def.name,
        ms_per_frame: total_ms / BENCH_MEASURED_SAMPLES as f64,
        iterations: BENCH_MEASURED_SAMPLES,
        total_ms,
    })
}

/// All predefined benchmarks.
pub(crate) fn bench_defs() -> Vec<BenchDef> {
    vec![
        // ── Rects (alpha) ──────────────────────────────────────────────
        BenchDef {
            name: "Rect - 5×5 - Solid",
            description: "rendering small semi-transparent rectangles",
            category: "Rects (alpha)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                calibrated_value: 600_000,
            }),
            params: &[
                (ParamId::NumRects, 600_000.0),
                (ParamId::RectSize, 5.0),
                (ParamId::PaintMode, 0.0),
                (ParamId::Rotated, 0.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Rect - 50×50 - Solid",
            description: "rendering medium semi-transparent rectangles",
            category: "Rects (alpha)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                calibrated_value: 380_000,
            }),
            params: &[
                (ParamId::NumRects, 380_000.0),
                (ParamId::RectSize, 50.0),
                (ParamId::PaintMode, 0.0),
                (ParamId::Rotated, 0.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Rect - 200×200 - Solid",
            description: "rendering large semi-transparent rectangles",
            category: "Rects (alpha)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                calibrated_value: 53_000,
            }),
            params: &[
                (ParamId::NumRects, 53_000.0),
                (ParamId::RectSize, 200.0),
                (ParamId::PaintMode, 0.0),
                (ParamId::Rotated, 0.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        // ── Rects (alpha, low overdraw) ──────────────────────────────
        // TargetOverlap keeps the average per-pixel overlap ratio constant
        // as NumRects scales with preset: rect size shrinks to compensate.
        // Low overlap means dest.a never fully saturates. This has been found to
        // have a large impact on pipeline architecture.
        BenchDef {
            name: "Rect - 2x Overlap",
            description: "alpha rects, ~2x avg per-pixel overlap — rect size adapts to viewport",
            category: "Rects (alpha, low overdraw)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                calibrated_value: 380_000,
            }),
            params: &[
                (ParamId::NumRects, 380_000.0),
                (ParamId::PaintMode, 0.0),
                (ParamId::Rotated, 0.0),
                (ParamId::Opaque, 0.0),
                (ParamId::TargetOverlap, 2.0),
            ],
        },
        BenchDef {
            name: "Rect - 4x Overlap",
            description: "alpha rects, ~4x avg per-pixel overlap — rect size adapts to viewport",
            category: "Rects (alpha, low overdraw)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                calibrated_value: 380_000,
            }),
            params: &[
                (ParamId::NumRects, 380_000.0),
                (ParamId::PaintMode, 0.0),
                (ParamId::Rotated, 0.0),
                (ParamId::Opaque, 0.0),
                (ParamId::TargetOverlap, 4.0),
            ],
        },
        // ── Rects (opaque) ─────────────────────────────────────────────
        BenchDef {
            name: "Rect - 5×5 - Solid (opaque)",
            description: "rendering small fully opaque rectangles",
            category: "Rects (opaque)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                calibrated_value: 600_000,
            }),
            params: &[
                (ParamId::NumRects, 600_000.0),
                (ParamId::RectSize, 5.0),
                (ParamId::PaintMode, 0.0),
                (ParamId::Rotated, 0.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        BenchDef {
            name: "Rect - 50×50 - Solid (opaque)",
            description: "rendering medium fully opaque rectangles",
            category: "Rects (opaque)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                calibrated_value: 380_000,
            }),
            params: &[
                (ParamId::NumRects, 380_000.0),
                (ParamId::RectSize, 50.0),
                (ParamId::PaintMode, 0.0),
                (ParamId::Rotated, 0.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        BenchDef {
            name: "Rect - 200×200 - Solid (opaque)",
            description: "rendering large fully opaque rectangles",
            category: "Rects (opaque)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                calibrated_value: 53_000,
            }),
            params: &[
                (ParamId::NumRects, 53_000.0),
                (ParamId::RectSize, 200.0),
                (ParamId::PaintMode, 0.0),
                (ParamId::Rotated, 0.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        // ── Images (alpha) ─────────────────────────────────────────────
        BenchDef {
            name: "Rect - 200×200 - Image - Nearest",
            description: "rendering transparent images with NN sampling",
            category: "Images (alpha)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                calibrated_value: 30_000,
            }),
            params: &[
                (ParamId::NumRects, 30_000.0),
                (ParamId::RectSize, 200.0),
                (ParamId::PaintMode, 2.0),
                (ParamId::Rotated, 0.0),
                (ParamId::ImageFilter, 0.0),
                (ParamId::ImageOpaque, 0.0),
            ],
        },
        BenchDef {
            name: "Rect - 200×200 - Image - Bilinear",
            description: "rendering transparent images with bilinear sampling",
            category: "Images (alpha)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                calibrated_value: 24_000,
            }),
            params: &[
                (ParamId::NumRects, 24_000.0),
                (ParamId::RectSize, 200.0),
                (ParamId::PaintMode, 2.0),
                (ParamId::Rotated, 0.0),
                (ParamId::ImageFilter, 1.0),
                (ParamId::ImageOpaque, 0.0),
            ],
        },
        // ── Images (alpha, low overdraw) ──────────────────────────────
        // Image rects with alpha go entirely through the alpha pass (atlas
        // textures have transparency).
        BenchDef {
            name: "Image - 2x Overlap - Nearest",
            description: "alpha images, ~2x avg overlap, NN sampling — rect size adapts to viewport",
            category: "Images (alpha, low overdraw)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                calibrated_value: 30_000,
            }),
            params: &[
                (ParamId::NumRects, 30_000.0),
                (ParamId::PaintMode, 2.0),
                (ParamId::Rotated, 0.0),
                (ParamId::ImageFilter, 0.0),
                (ParamId::ImageOpaque, 0.0),
                (ParamId::TargetOverlap, 2.0),
            ],
        },
        BenchDef {
            name: "Image - 4x Overlap - Nearest",
            description: "alpha images, ~4x avg overlap, NN sampling — rect size adapts to viewport",
            category: "Images (alpha, low overdraw)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                calibrated_value: 30_000,
            }),
            params: &[
                (ParamId::NumRects, 30_000.0),
                (ParamId::PaintMode, 2.0),
                (ParamId::Rotated, 0.0),
                (ParamId::ImageFilter, 0.0),
                (ParamId::ImageOpaque, 0.0),
                (ParamId::TargetOverlap, 4.0),
            ],
        },
        // ── Images (opaque) ────────────────────────────────────────────
        BenchDef {
            name: "Rect - 200×200 - Opaque Image - Nearest",
            description: "rendering opaque images with NN sampling",
            category: "Images (opaque)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                calibrated_value: 31_000,
            }),
            params: &[
                (ParamId::NumRects, 31_000.0),
                (ParamId::RectSize, 200.0),
                (ParamId::PaintMode, 2.0),
                (ParamId::Rotated, 0.0),
                (ParamId::ImageFilter, 0.0),
                (ParamId::ImageOpaque, 1.0),
            ],
        },
        BenchDef {
            name: "Rect - 200×200 - Opaque Image - Bilinear",
            description: "rendering opaque images with bilinear sampling",
            category: "Images (opaque)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                calibrated_value: 24_000,
            }),
            params: &[
                (ParamId::NumRects, 24_000.0),
                (ParamId::RectSize, 200.0),
                (ParamId::PaintMode, 2.0),
                (ParamId::Rotated, 0.0),
                (ParamId::ImageFilter, 1.0),
                (ParamId::ImageOpaque, 1.0),
            ],
        },
        // BenchDef {
        //     name: "Rect - 200×200 - Opaque Image (draw_image) - Nearest",
        //     description: "rendering images via draw_image API (GPU fast path on hybrid)",
        //     category: "Images (opaque)",
        //     scene_id: SceneId::Rect,
        //     scale: Some(BenchScale {
        //         param: ParamId::NumRects,
        //         calibrated_value: 35_000,
        //     }),
        //     params: &[
        //         (ParamId::NumRects, 35_000.0),
        //         (ParamId::RectSize, 200.0),
        //         (ParamId::PaintMode, 2.0),
        //         (ParamId::Rotated, 0.0),
        //         (ParamId::ImageFilter, 0.0),
        //         (ParamId::ImageOpaque, 1.0),
        //         (ParamId::UseDrawImage, 1.0),
        //     ],
        // },
        // BenchDef {
        //     name: "Rect - 200×200 - Opaque Image (draw_image) - Bilinear",
        //     description: "rendering images via draw_image API with bilinear (GPU fast path on hybrid)",
        //     category: "Images (opaque)",
        //     scene_id: SceneId::Rect,
        //     scale: Some(BenchScale {
        //         param: ParamId::NumRects,
        //         calibrated_value: 34_000,
        //     }),
        //     params: &[
        //         (ParamId::NumRects, 34_000.0),
        //         (ParamId::RectSize, 200.0),
        //         (ParamId::PaintMode, 2.0),
        //         (ParamId::Rotated, 0.0),
        //         (ParamId::ImageFilter, 1.0),
        //         (ParamId::ImageOpaque, 1.0),
        //         (ParamId::UseDrawImage, 1.0),
        //     ],
        // },
        // ── Strokes (alpha) ────────────────────────────────────────────
        BenchDef {
            name: "Stroked Lines - 3px",
            description: "rendering semi-transparent lines with small stroke width",
            category: "Strokes (alpha)",
            scene_id: SceneId::Strokes,
            scale: Some(BenchScale {
                param: ParamId::NumStrokes,
                calibrated_value: 18_500,
            }),
            params: &[
                (ParamId::NumStrokes, 18_500.0),
                (ParamId::CurveType, 0.0),
                (ParamId::StrokeWidth, 3.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Stroked Lines - 20px",
            description: "rendering semi-transparent lines with large stroke width",
            category: "Strokes (alpha)",
            scene_id: SceneId::Strokes,
            scale: Some(BenchScale {
                param: ParamId::NumStrokes,
                calibrated_value: 13_200,
            }),
            params: &[
                (ParamId::NumStrokes, 13_200.0),
                (ParamId::CurveType, 0.0),
                (ParamId::StrokeWidth, 20.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Stroked Quads - 3px",
            description: "rendering semi-transparent quads with small stroke width",
            category: "Strokes (alpha)",
            scene_id: SceneId::Strokes,
            scale: Some(BenchScale {
                param: ParamId::NumStrokes,
                calibrated_value: 6_900,
            }),
            params: &[
                (ParamId::NumStrokes, 6_900.0),
                (ParamId::CurveType, 1.0),
                (ParamId::StrokeWidth, 3.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Stroked Quads - 20px",
            description: "rendering semi-transparent quads with large stroke width",
            category: "Strokes (alpha)",
            scene_id: SceneId::Strokes,
            scale: Some(BenchScale {
                param: ParamId::NumStrokes,
                calibrated_value: 5_100,
            }),
            params: &[
                (ParamId::NumStrokes, 5_100.0),
                (ParamId::CurveType, 1.0),
                (ParamId::StrokeWidth, 20.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Stroked Cubics - 3px",
            description: "rendering semi-transparent cubics with small stroke width",
            category: "Strokes (alpha)",
            scene_id: SceneId::Strokes,
            scale: Some(BenchScale {
                param: ParamId::NumStrokes,
                calibrated_value: 5_000,
            }),
            params: &[
                (ParamId::NumStrokes, 5_000.0),
                (ParamId::CurveType, 2.0),
                (ParamId::StrokeWidth, 3.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Stroked Cubics - 20px",
            description: "rendering semi-transparent cubics with large stroke width",
            category: "Strokes (alpha)",
            scene_id: SceneId::Strokes,
            scale: Some(BenchScale {
                param: ParamId::NumStrokes,
                calibrated_value: 3_500,
            }),
            params: &[
                (ParamId::NumStrokes, 3_500.0),
                (ParamId::CurveType, 2.0),
                (ParamId::StrokeWidth, 20.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        // ── Strokes (opaque) ───────────────────────────────────────────
        BenchDef {
            name: "Stroked Lines - 3px (opaque)",
            description: "rendering opaque lines with small stroke width",
            category: "Strokes (opaque)",
            scene_id: SceneId::Strokes,
            scale: Some(BenchScale {
                param: ParamId::NumStrokes,
                calibrated_value: 18_500,
            }),
            params: &[
                (ParamId::NumStrokes, 18_500.0),
                (ParamId::CurveType, 0.0),
                (ParamId::StrokeWidth, 3.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        BenchDef {
            name: "Stroked Lines - 20px (opaque)",
            description: "rendering opaque lines with large stroke width",
            category: "Strokes (opaque)",
            scene_id: SceneId::Strokes,
            scale: Some(BenchScale {
                param: ParamId::NumStrokes,
                calibrated_value: 13_200,
            }),
            params: &[
                (ParamId::NumStrokes, 13_200.0),
                (ParamId::CurveType, 0.0),
                (ParamId::StrokeWidth, 20.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        BenchDef {
            name: "Stroked Cubics - 3px (opaque)",
            description: "rendering opaque cubics with small stroke width",
            category: "Strokes (opaque)",
            scene_id: SceneId::Strokes,
            scale: Some(BenchScale {
                param: ParamId::NumStrokes,
                calibrated_value: 5_000,
            }),
            params: &[
                (ParamId::NumStrokes, 5_000.0),
                (ParamId::CurveType, 2.0),
                (ParamId::StrokeWidth, 3.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        BenchDef {
            name: "Stroked Cubics - 20px (opaque)",
            description: "rendering opaque cubics with large stroke width",
            category: "Strokes (opaque)",
            scene_id: SceneId::Strokes,
            scale: Some(BenchScale {
                param: ParamId::NumStrokes,
                calibrated_value: 3_500,
            }),
            params: &[
                (ParamId::NumStrokes, 3_500.0),
                (ParamId::CurveType, 2.0),
                (ParamId::StrokeWidth, 20.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        BenchDef {
            name: "Polyline",
            description: "rendering paths bottlenecked by tiling and strip rendering",
            category: "Fills",
            scene_id: SceneId::Polyline,
            scale: Some(BenchScale {
                param: ParamId::NumVertices,
                calibrated_value: 2_200,
            }),
            params: &[(ParamId::NumVertices, 2200.0)],
        },
        BenchDef {
            name: "Ghostscript Tiger",
            description: "rendering simple vector graphics",
            category: "Vector Graphics",
            scene_id: SceneId::Svg,
            scale: None,
            params: &[(ParamId::SvgAsset, 0.0)],
        },
        BenchDef {
            name: "Coat of Arms",
            description: "rendering simple vector graphics",
            category: "Vector Graphics",
            scene_id: SceneId::Svg,
            scale: None,
            params: &[(ParamId::SvgAsset, 1.0)],
        },
        BenchDef {
            name: "Heraldry",
            description: "rendering simple vector graphics",
            category: "Vector Graphics",
            scene_id: SceneId::Svg,
            scale: None,
            params: &[(ParamId::SvgAsset, 2.0)],
        },
        // ── Clip Paths (alpha) ─────────────────────────────────────────
        BenchDef {
            name: "Rect - 400px - Global `clip_path`",
            description: "rendering many semi-transparent paths with a single clip path",
            category: "Clip Paths (alpha)",
            scene_id: SceneId::Clip,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                calibrated_value: 2_100,
            }),
            params: &[
                (ParamId::NumRects, 2_100.0),
                (ParamId::RectSize, 400.0),
                (ParamId::ClipMode, 1.0),
                (ParamId::ClipMethod, 0.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Rect - 400px - Global `clip_layer`",
            description: "rendering many semi-transparent paths with a single clip layer",
            category: "Clip Paths (alpha)",
            scene_id: SceneId::Clip,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                calibrated_value: 2_100,
            }),
            params: &[
                (ParamId::NumRects, 2_100.0),
                (ParamId::RectSize, 400.0),
                (ParamId::ClipMode, 1.0),
                (ParamId::ClipMethod, 1.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Rect - 200px - Per-shape `clip_path`",
            description: "rendering many semi-transparent paths with many clip paths",
            category: "Clip Paths (alpha)",
            scene_id: SceneId::Clip,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                calibrated_value: 930,
            }),
            params: &[
                (ParamId::NumRects, 930.0),
                (ParamId::RectSize, 200.0),
                (ParamId::ClipMode, 2.0),
                (ParamId::ClipMethod, 0.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Rect - 200px - Per-shape `clip_layer`",
            description: "rendering many semi-transparent paths with many clip layers",
            category: "Clip Paths (alpha)",
            scene_id: SceneId::Clip,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                calibrated_value: 930,
            }),
            params: &[
                (ParamId::NumRects, 930.0),
                (ParamId::RectSize, 200.0),
                (ParamId::ClipMode, 2.0),
                (ParamId::ClipMethod, 1.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        // ── Clip Paths (opaque) ────────────────────────────────────────
        BenchDef {
            name: "Rect - 400px - Global `clip_path` (opaque)",
            description: "rendering many opaque paths with a single clip path",
            category: "Clip Paths (opaque)",
            scene_id: SceneId::Clip,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                calibrated_value: 2_100,
            }),
            params: &[
                (ParamId::NumRects, 2_100.0),
                (ParamId::RectSize, 400.0),
                (ParamId::ClipMode, 1.0),
                (ParamId::ClipMethod, 0.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        BenchDef {
            name: "Rect - 200px - Per-shape `clip_path` (opaque)",
            description: "rendering many opaque paths with many clip paths",
            category: "Clip Paths (opaque)",
            scene_id: SceneId::Clip,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                calibrated_value: 930,
            }),
            params: &[
                (ParamId::NumRects, 930.0),
                (ParamId::RectSize, 200.0),
                (ParamId::ClipMode, 2.0),
                (ParamId::ClipMethod, 0.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        // ── Text (alpha) ───────────────────────────────────────────────
        BenchDef {
            name: "Text - 8px",
            description: "rendering small semi-transparent text",
            category: "Text (alpha)",
            scene_id: SceneId::Text,
            scale: Some(BenchScale {
                param: ParamId::NumRuns,
                calibrated_value: 2_900,
            }),
            params: &[
                (ParamId::NumRuns, 2_900.0),
                (ParamId::FontSize, 8.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Text - 24px",
            description: "rendering medium-sized semi-transparent text",
            category: "Text (alpha)",
            scene_id: SceneId::Text,
            scale: Some(BenchScale {
                param: ParamId::NumRuns,
                calibrated_value: 2_200,
            }),
            params: &[
                (ParamId::NumRuns, 2_200.0),
                (ParamId::FontSize, 24.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Text - 60px",
            description: "rendering large semi-transparent text",
            category: "Text (alpha)",
            scene_id: SceneId::Text,
            scale: Some(BenchScale {
                param: ParamId::NumRuns,
                calibrated_value: 1_300,
            }),
            params: &[
                (ParamId::NumRuns, 1_300.0),
                (ParamId::FontSize, 60.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        // ── Text (opaque) ──────────────────────────────────────────────
        BenchDef {
            name: "Text - 8px (opaque)",
            description: "rendering small opaque text",
            category: "Text (opaque)",
            scene_id: SceneId::Text,
            scale: Some(BenchScale {
                param: ParamId::NumRuns,
                calibrated_value: 2_900,
            }),
            params: &[
                (ParamId::NumRuns, 2_900.0),
                (ParamId::FontSize, 8.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        BenchDef {
            name: "Text - 24px (opaque)",
            description: "rendering medium-sized opaque text",
            category: "Text (opaque)",
            scene_id: SceneId::Text,
            scale: Some(BenchScale {
                param: ParamId::NumRuns,
                calibrated_value: 2_200,
            }),
            params: &[
                (ParamId::NumRuns, 2_200.0),
                (ParamId::FontSize, 24.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        BenchDef {
            name: "Text - 60px (opaque)",
            description: "rendering large opaque text",
            category: "Text (opaque)",
            scene_id: SceneId::Text,
            scale: Some(BenchScale {
                param: ParamId::NumRuns,
                calibrated_value: 1_300,
            }),
            params: &[
                (ParamId::NumRuns, 1_300.0),
                (ParamId::FontSize, 60.0),
                (ParamId::Opaque, 1.0),
            ],
        },
    ]
}
