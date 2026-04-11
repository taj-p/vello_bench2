//! Clip path benchmark backend.

#![allow(
    clippy::cast_possible_truncation,
    reason = "truncation has no appreciable impact in this benchmark"
)]

use std::f64::consts::{FRAC_PI_2, PI};

use super::{BenchScene, Param, ParamId, ParamKind, SceneId, bounce, delta_time};
use crate::backend::Backend;
use crate::resource_store::ResourceStore;
use crate::rng::Rng;
use vello_common::kurbo::{Affine, BezPath, Point, Rect, Vec2};
use vello_common::peniko::Color;

/// An animated rectangle with position, velocity, color.
#[derive(Debug)]
struct AnimatedRect {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    color: Color,
}

/// Benchmark scene for measuring clip path performance.
#[derive(Debug)]
pub struct ClipScene {
    num_rects: usize,
    rect_size: f64,
    speed: f64,
    /// 0 = push_clip_path, 1 = push_clip_layer
    clip_method: u32,
    /// 0 = No clipping, 1 = Single global clip, 2 = Per-shape clip
    clip_mode: u32,
    /// When true, fill colors use alpha 255 instead of 200.
    opaque: bool,
    rects: Vec<AnimatedRect>,
    rng: Rng,
    last_time: f64,
}

impl ClipScene {
    /// Create a new clip path benchmark backend.
    pub fn new() -> Self {
        Self {
            num_rects: 500,
            rect_size: 250.0,
            speed: 5.0,
            clip_method: 0,
            clip_mode: 1,
            opaque: false,
            rects: Vec::new(),
            rng: Rng::new(0xC11D_CAFE),
            last_time: 0.0,
        }
    }

    fn ensure_rects(&mut self, w: f64, h: f64) {
        let alpha = if self.opaque { 255 } else { 200 };
        if self.rects.len() < self.num_rects {
            while self.rects.len() < self.num_rects {
                self.rects.push(random_rect(&mut self.rng, w, h, alpha));
            }
        } else if self.rects.len() > self.num_rects {
            self.rects.truncate(self.num_rects);
        }
    }
}

fn random_rect(rng: &mut Rng, w: f64, h: f64, alpha: u8) -> AnimatedRect {
    AnimatedRect {
        x: rng.f64() * w,
        y: rng.f64() * h,
        vx: (rng.f64() - 0.5) * 200.0,
        vy: (rng.f64() - 0.5) * 200.0,
        color: rng.color(alpha),
    }
}

const STAR_POINTS: usize = 20;

/// Generate a star BezPath centered at `center` with `STAR_POINTS` points,
/// alternating between `outer` and `inner` radii.
fn star_path(center: Point, inner: f64, outer: f64) -> BezPath {
    let n = STAR_POINTS;
    let mut path = BezPath::new();
    let start_angle = -FRAC_PI_2;
    path.move_to(center + outer * Vec2::from_angle(start_angle));
    for i in 1..n * 2 {
        let th = start_angle + i as f64 * PI / n as f64;
        let r = if i % 2 == 0 { outer } else { inner };
        path.line_to(center + r * Vec2::from_angle(th));
    }
    path.close_path();
    path
}

impl BenchScene for ClipScene {
    fn scene_id(&self) -> SceneId {
        SceneId::Clip
    }

    fn name(&self) -> &str {
        "Clip Paths"
    }

    fn params(&self) -> Vec<Param> {
        vec![
            Param {
                id: ParamId::NumRects,
                label: "Rectangles",
                kind: ParamKind::Slider {
                    min: 1.0,
                    max: 5_000.0,
                    step: 1.0,
                },
                value: self.num_rects as f64,
            },
            Param {
                id: ParamId::RectSize,
                label: "Rect Size",
                kind: ParamKind::Slider {
                    min: 5.0,
                    max: 500.0,
                    step: 1.0,
                },
                value: self.rect_size,
            },
            Param {
                id: ParamId::ClipMode,
                label: "Clip Mode",
                kind: ParamKind::Select(vec![("None", 0.0), ("Global", 1.0), ("Per-Shape", 2.0)]),
                value: self.clip_mode as f64,
            },
            Param {
                id: ParamId::ClipMethod,
                label: "Clip Method",
                kind: ParamKind::Select(vec![("clip_path", 0.0), ("clip_layer", 1.0)]),
                value: self.clip_method as f64,
            },
            Param {
                id: ParamId::Opaque,
                label: "Opaque",
                kind: ParamKind::Select(vec![("No", 0.0), ("Yes", 1.0)]),
                value: if self.opaque { 1.0 } else { 0.0 },
            },
        ]
    }

    fn set_param(&mut self, param: ParamId, value: f64) {
        match param {
            ParamId::NumRects => self.num_rects = value as usize,
            ParamId::RectSize => self.rect_size = value,
            ParamId::ClipMode => self.clip_mode = value as u32,
            ParamId::ClipMethod => self.clip_method = value as u32,
            ParamId::Opaque => {
                let new_val = value >= 0.5;
                if new_val != self.opaque {
                    self.opaque = new_val;
                    self.rects.clear();
                }
            }
            _ => {}
        }
    }

    fn render(
        &mut self,
        backend: &mut dyn Backend,
        _resources: &mut ResourceStore,
        width: u32,
        height: u32,
        time: f64,
        view: Affine,
    ) {
        let w = width as f64;
        let h = height as f64;

        self.ensure_rects(w, h);

        let dt = delta_time(&mut self.last_time, time, self.speed);

        let size = self.rect_size;
        let use_clip_layer = self.clip_method == 1;

        backend.set_transform(view);

        // Mode 1: single global star clip covering the viewport.
        if self.clip_mode == 1 {
            let cx = w / 2.0;
            let cy = h / 2.0;
            let outer = (cx * cx + cy * cy).sqrt();
            let inner = outer * 0.4;
            let clip = star_path(Point::new(cx, cy), inner, outer);
            if use_clip_layer {
                backend.push_clip_layer(&clip);
            } else {
                backend.push_clip_path(&clip);
            }
        }

        for r in &mut self.rects {
            r.x += r.vx * dt;
            r.y += r.vy * dt;
            bounce(&mut r.x, &mut r.vx, w);
            bounce(&mut r.y, &mut r.vy, h);

            // Mode 2: per-shape star clip.
            if self.clip_mode == 2 {
                let cx = r.x + size / 2.0;
                let cy = r.y + size / 2.0;
                let outer = size / 2.0;
                let inner = outer * 0.4;
                let clip = star_path(Point::new(cx, cy), inner, outer);
                if use_clip_layer {
                    backend.push_clip_layer(&clip);
                } else {
                    backend.push_clip_path(&clip);
                }
            }

            let rect = Rect::new(r.x, r.y, r.x + size, r.y + size);
            backend.set_paint(r.color.into());
            backend.fill_rect(&rect);

            if self.clip_mode == 2 {
                if use_clip_layer {
                    backend.pop_layer();
                } else {
                    backend.pop_clip_path();
                }
            }
        }

        if self.clip_mode == 1 {
            if use_clip_layer {
                backend.pop_layer();
            } else {
                backend.pop_clip_path();
            }
        }

        backend.reset_transform();
    }
}
