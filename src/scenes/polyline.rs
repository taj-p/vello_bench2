//! Filled polyline benchmark scene.

#![allow(
    clippy::cast_possible_truncation,
    reason = "truncation has no appreciable impact in this benchmark"
)]

use super::{BenchScene, Param, ParamKind, bounce, delta_time};
use crate::backend::{Backend, DrawContext};
use crate::rng::Rng;
use vello_common::kurbo::{Affine, BezPath};
use vello_common::peniko::{Color, Fill};

/// An animated vertex with position and velocity.
#[derive(Debug)]
struct AnimVertex {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
}

/// Benchmark scene that fills a single polyline path with many vertices.
#[derive(Debug)]
pub struct PolylineScene {
    num_vertices: usize,
    speed: f64,
    vertices: Vec<AnimVertex>,
    rng: Rng,
    last_time: f64,
}

impl PolylineScene {
    /// Create a new polyline benchmark scene.
    pub fn new() -> Self {
        Self {
            num_vertices: 100,
            speed: 5.0,
            vertices: Vec::new(),
            rng: Rng::new(0xF00D_CAFE),
            last_time: 0.0,
        }
    }

    fn ensure_vertices(&mut self, w: f64, h: f64) {
        if self.vertices.len() < self.num_vertices {
            while self.vertices.len() < self.num_vertices {
                self.vertices.push(AnimVertex {
                    x: self.rng.f64() * w,
                    y: self.rng.f64() * h,
                    vx: (self.rng.f64() - 0.5) * 150.0,
                    vy: (self.rng.f64() - 0.5) * 150.0,
                });
            }
        } else if self.vertices.len() > self.num_vertices {
            self.vertices.truncate(self.num_vertices);
        }
    }
}

impl BenchScene for PolylineScene {
    fn name(&self) -> &str {
        "Polyline"
    }

    fn params(&self) -> Vec<Param> {
        vec![Param {
            name: "num_vertices",
            label: "Vertices",
            kind: ParamKind::Slider {
                min: 20.0,
                max: 10000.0,
                step: 1.0,
            },
            value: self.num_vertices as f64,
        }]
    }

    fn set_param(&mut self, name: &str, value: f64) {
        match name {
            "num_vertices" => self.num_vertices = (value as usize).max(3),
            _ => {}
        }
    }

    fn render(
        &mut self,
        scene: &mut DrawContext,
        _backend: &mut Backend,
        width: u32,
        height: u32,
        time: f64,
        view: Affine,
    ) {
        let w = width as f64;
        let h = height as f64;

        self.ensure_vertices(w, h);

        let dt = delta_time(&mut self.last_time, time, self.speed);

        for v in &mut self.vertices {
            v.x += v.vx * dt;
            v.y += v.vy * dt;
            bounce(&mut v.x, &mut v.vx, w);
            bounce(&mut v.y, &mut v.vy, h);
        }

        let mut path = BezPath::new();
        path.move_to((self.vertices[0].x, self.vertices[0].y));
        for v in &self.vertices[1..] {
            path.line_to((v.x, v.y));
        }
        path.close_path();

        scene.set_transform(view);
        scene.set_paint(Color::from_rgba8(66, 135, 245, 180));
        scene.set_fill_rule(Fill::EvenOdd);
        scene.fill_path(&path);
        scene.reset_transform();
    }
}
