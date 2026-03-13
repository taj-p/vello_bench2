//! Animated strokes benchmark scene (lines, quadratic, cubic).

#![allow(
    clippy::cast_possible_truncation,
    reason = "truncation has no appreciable impact in this benchmark"
)]

use super::{BenchScene, Param, ParamKind, bounce, delta_time};
use crate::rng::Rng;
use vello_common::kurbo::{Affine, BezPath, Cap, Stroke};
use vello_common::peniko::Color;
use vello_hybrid::{Scene, WebGlRenderer};

/// A static offset point relative to the stroke's origin.
#[derive(Debug, Clone)]
struct OffsetPoint {
    dx: f64,
    dy: f64,
}

/// An animated stroke that moves as a whole unit.
/// `x, y` is the origin that animates; `offsets` are static control points relative to origin.
#[derive(Debug)]
struct AnimatedStroke {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    /// Static control point offsets from the origin, generated per-segment.
    /// The first offset is always (0, 0) for the move-to point.
    offsets: Vec<OffsetPoint>,
    color: Color,
}

/// Benchmark scene that strokes many animated random paths.
#[derive(Debug)]
pub struct StrokesScene {
    num_strokes: usize,
    stroke_width: f64,
    /// 0 = Butt, 1 = Square, 2 = Round
    cap: u32,
    /// 0 = Line, 1 = Quadratic, 2 = Cubic
    curve_type: u32,
    /// Number of segments per stroke path.
    segments: usize,
    speed: f64,
    strokes: Vec<AnimatedStroke>,
    rng: Rng,
    last_time: f64,
    /// Track config that requires offset regeneration.
    gen_curve_type: u32,
}

impl StrokesScene {
    /// Create a new strokes benchmark scene.
    pub fn new() -> Self {
        Self {
            num_strokes: 200,
            stroke_width: 3.0,
            cap: 0,
            curve_type: 0,
            segments: 1,
            speed: 5.0,
            strokes: Vec::new(),
            rng: Rng::new(0xBEEF_CAFE),
            last_time: 0.0,
            gen_curve_type: 0,
        }
    }

    /// Ensure the stroke list matches the desired count and config.
    fn ensure_strokes(&mut self, w: f64, h: f64) {
        let type_changed = self.gen_curve_type != self.curve_type;

        if type_changed {
            // Curve type changed — regenerate all offsets (but preserve positions/velocities).
            for s in &mut self.strokes {
                let pts_per_seg = points_per_segment(self.curve_type);
                let total_pts = 1 + self.segments * pts_per_seg;
                regenerate_offsets(&mut self.rng, s, total_pts);
            }
            self.gen_curve_type = self.curve_type;
        }

        // Grow or shrink stroke count.
        if self.strokes.len() < self.num_strokes {
            let pts_per_seg = points_per_segment(self.curve_type);
            let total_pts = 1 + self.segments * pts_per_seg;
            while self.strokes.len() < self.num_strokes {
                self.strokes
                    .push(random_stroke(&mut self.rng, w, h, total_pts));
            }
        } else if self.strokes.len() > self.num_strokes {
            self.strokes.truncate(self.num_strokes);
        }

        // Adjust segments per stroke (extend or truncate offsets).
        let pts_per_seg = points_per_segment(self.curve_type);
        let total_pts = 1 + self.segments * pts_per_seg;
        for s in &mut self.strokes {
            if s.offsets.len() < total_pts {
                // Extend with new offsets continuing from the last point.
                extend_offsets(&mut self.rng, s, total_pts);
            } else if s.offsets.len() > total_pts {
                s.offsets.truncate(total_pts);
            }
        }
    }
}

/// How many new points each segment adds (line=1, quad=2, cubic=3).
fn points_per_segment(curve_type: u32) -> usize {
    match curve_type {
        1 => 2,
        2 => 3,
        _ => 1,
    }
}

const SPREAD: f64 = 200.0;

fn random_offset_near(rng: &mut Rng, prev: &OffsetPoint) -> OffsetPoint {
    OffsetPoint {
        dx: prev.dx + (rng.f64() - 0.5) * SPREAD,
        dy: prev.dy + (rng.f64() - 0.5) * SPREAD,
    }
}

fn random_stroke(rng: &mut Rng, w: f64, h: f64, total_pts: usize) -> AnimatedStroke {
    let x = rng.f64() * w;
    let y = rng.f64() * h;
    let vx = (rng.f64() - 0.5) * 200.0;
    let vy = (rng.f64() - 0.5) * 200.0;

    let mut offsets = Vec::with_capacity(total_pts);
    offsets.push(OffsetPoint { dx: 0.0, dy: 0.0 });
    for i in 1..total_pts {
        offsets.push(random_offset_near(rng, &offsets[i - 1]));
    }

    AnimatedStroke {
        x,
        y,
        vx,
        vy,
        offsets,
        color: rng.color(150),
    }
}

/// Regenerate all offsets for a stroke (when curve type changes).
fn regenerate_offsets(rng: &mut Rng, s: &mut AnimatedStroke, total_pts: usize) {
    s.offsets.clear();
    s.offsets.push(OffsetPoint { dx: 0.0, dy: 0.0 });
    for i in 1..total_pts {
        s.offsets.push(random_offset_near(rng, &s.offsets[i - 1]));
    }
}

/// Extend offsets to reach `total_pts`, continuing from the last existing offset.
fn extend_offsets(rng: &mut Rng, s: &mut AnimatedStroke, total_pts: usize) {
    while s.offsets.len() < total_pts {
        let last = s.offsets.last().unwrap().clone();
        s.offsets.push(random_offset_near(rng, &last));
    }
}

impl BenchScene for StrokesScene {
    fn name(&self) -> &str {
        "Strokes"
    }

    fn params(&self) -> Vec<Param> {
        vec![
            Param {
                name: "num_strokes",
                label: "Strokes",
                kind: ParamKind::Slider {
                    min: 1.0,
                    max: 3_000.0,
                    step: 1.0,
                },
                value: self.num_strokes as f64,
            },
            Param {
                name: "curve_type",
                label: "Curve",
                kind: ParamKind::Select(vec![("Line", 0.0), ("Quadratic", 1.0), ("Cubic", 2.0)]),
                value: self.curve_type as f64,
            },
            Param {
                name: "segments",
                label: "Segments",
                kind: ParamKind::Slider {
                    min: 1.0,
                    max: 50.0,
                    step: 1.0,
                },
                value: self.segments as f64,
            },
            Param {
                name: "stroke_width",
                label: "Stroke Width",
                kind: ParamKind::Slider {
                    min: 0.5,
                    max: 50.0,
                    step: 0.5,
                },
                value: self.stroke_width,
            },
            Param {
                name: "cap",
                label: "Cap",
                kind: ParamKind::Select(vec![("Butt", 0.0), ("Square", 1.0), ("Round", 2.0)]),
                value: self.cap as f64,
            },
        ]
    }

    fn set_param(&mut self, name: &str, value: f64) {
        match name {
            "num_strokes" => self.num_strokes = value as usize,
            "curve_type" => self.curve_type = value as u32,
            "segments" => self.segments = (value as usize).max(1),
            "stroke_width" => self.stroke_width = value,
            "cap" => self.cap = value as u32,
            _ => {}
        }
    }

    fn render(
        &mut self,
        scene: &mut Scene,
        _renderer: &mut WebGlRenderer,
        width: u32,
        height: u32,
        time: f64,
        view: Affine,
    ) {
        let w = width as f64;
        let h = height as f64;

        self.ensure_strokes(w, h);

        let dt = delta_time(&mut self.last_time, time, self.speed);

        let cap = match self.cap {
            1 => Cap::Square,
            2 => Cap::Round,
            _ => Cap::Butt,
        };

        scene.set_transform(view);
        scene.set_stroke(Stroke::new(self.stroke_width).with_caps(cap));

        let curve_type = self.curve_type;
        let pts_per_seg = points_per_segment(curve_type);
        let mut path = BezPath::new();
        for s in &mut self.strokes {
            // Move the whole stroke as one unit.
            s.x += s.vx * dt;
            s.y += s.vy * dt;
            bounce(&mut s.x, &mut s.vx, w);
            bounce(&mut s.y, &mut s.vy, h);

            let ox = s.x;
            let oy = s.y;

            path.move_to((ox + s.offsets[0].dx, oy + s.offsets[0].dy));
            let mut i = 1;
            while i + pts_per_seg <= s.offsets.len() {
                match curve_type {
                    1 => {
                        path.quad_to(
                            (ox + s.offsets[i].dx, oy + s.offsets[i].dy),
                            (ox + s.offsets[i + 1].dx, oy + s.offsets[i + 1].dy),
                        );
                        i += 2;
                    }
                    2 => {
                        path.curve_to(
                            (ox + s.offsets[i].dx, oy + s.offsets[i].dy),
                            (ox + s.offsets[i + 1].dx, oy + s.offsets[i + 1].dy),
                            (ox + s.offsets[i + 2].dx, oy + s.offsets[i + 2].dy),
                        );
                        i += 3;
                    }
                    _ => {
                        path.line_to((ox + s.offsets[i].dx, oy + s.offsets[i].dy));
                        i += 1;
                    }
                }
            }
            scene.set_paint(s.color);
            scene.stroke_path(&path);
            path.truncate(0);
        }

        scene.reset_transform();
    }
}
