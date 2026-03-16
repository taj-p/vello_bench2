//! Animated rectangles benchmark scene.

#![allow(
    clippy::cast_possible_truncation,
    reason = "truncation has no appreciable impact in this benchmark"
)]

use super::{BenchScene, Param, ParamKind, bounce, delta_time};
use crate::backend::{Backend, DrawContext, Pixmap};
use crate::rng::Rng;
use smallvec::smallvec;
use vello_common::kurbo::{Affine, Point, Rect};
use vello_common::paint::{Image, ImageId, ImageSource};
use vello_common::peniko::{
    Color, ColorStop, ColorStops, Extend, Gradient, ImageQuality, ImageSampler,
    LinearGradientPosition, RadialGradientPosition, SweepGradientPosition, color::DynamicColor,
    color::PremulRgba8,
};

const NUM_IMAGES: usize = 50;
const IMAGE_SIZE: u16 = 64;

/// A smoothly oscillating f32 value.
#[derive(Debug, Clone)]
struct Oscillator {
    speed: f32,
    phase: f32,
}

impl Oscillator {
    fn generate(rng: &mut Rng) -> Self {
        Self {
            speed: 0.04 + rng.f64() as f32 * 0.12,
            phase: rng.f64() as f32 * std::f32::consts::TAU,
        }
    }

    /// Returns a value in [-1, 1].
    fn sample(&self, frame: u64) -> f32 {
        (frame as f32 * self.speed + self.phase).sin()
    }
}

/// Per-color-channel oscillator for animated gradient stop colors.
#[derive(Debug, Clone)]
struct ColorOscillator {
    base: f32,
    speed: f32,
    phase: f32,
}

impl ColorOscillator {
    fn generate(rng: &mut Rng) -> Self {
        Self {
            base: 40.0 + rng.f64() as f32 * 175.0,
            // Fast: changes visibly every frame.
            speed: 0.06 + rng.f64() as f32 * 0.15,
            phase: rng.f64() as f32 * std::f32::consts::TAU,
        }
    }

    fn sample(&self, frame: u64) -> u8 {
        let t = frame as f32 * self.speed + self.phase;
        (self.base + t.sin() * 80.0).clamp(0.0, 255.0) as u8
    }
}

/// Animated color with 3 independent channel oscillators.
#[derive(Debug, Clone)]
struct AnimColor {
    r: ColorOscillator,
    g: ColorOscillator,
    b: ColorOscillator,
}

impl AnimColor {
    fn generate(rng: &mut Rng) -> Self {
        Self {
            r: ColorOscillator::generate(rng),
            g: ColorOscillator::generate(rng),
            b: ColorOscillator::generate(rng),
        }
    }

    fn sample(&self, frame: u64) -> Color {
        Color::from_rgba8(
            self.r.sample(frame),
            self.g.sample(frame),
            self.b.sample(frame),
            200,
        )
    }
}

/// Per-rect gradient animation state (geometry + colors).
#[derive(Debug, Clone)]
struct GradientAnim {
    /// Linear: animates the gradient line angle.
    angle: Oscillator,
    /// Radial: animates the end radius.
    radius: Oscillator,
    /// Sweep: animates the end angle.
    sweep: Oscillator,
    /// Animated stop colors — unique per rect, change every frame.
    color1: AnimColor,
    color2: AnimColor,
}

impl GradientAnim {
    fn generate(rng: &mut Rng) -> Self {
        Self {
            angle: Oscillator::generate(rng),
            radius: Oscillator::generate(rng),
            sweep: Oscillator::generate(rng),
            color1: AnimColor::generate(rng),
            color2: AnimColor::generate(rng),
        }
    }
}

/// An animated rectangle with position, velocity, color.
#[derive(Debug)]
struct AnimatedRect {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    color: Color,
    color2: Color,
    /// Per-rect gradient animation (geometry + colors).
    grad_anim: GradientAnim,
    /// Index into the image table (`0..NUM_IMAGES`).
    image_idx: usize,
    /// Rotation angle in radians (stable per rect).
    angle: f64,
}

/// Benchmark scene that renders many animated rectangles.
#[derive(Debug)]
pub struct RectScene {
    num_rects: usize,
    speed: f64,
    /// 0 = solid, 1 = gradient, 2 = image
    paint_mode: u32,
    rect_size: f64,
    rotated: bool,
    /// 0 = nearest-neighbor (Low), 1 = bilinear (Medium)
    image_filter: u32,
    /// Whether images are fully opaque (no alpha fade).
    image_opaque: bool,
    /// When true, gradient colors and positions animate every frame.
    dynamic_gradient: bool,
    /// 0 = linear, 1 = radial, 2 = sweep
    gradient_shape: u32,
    rects: Vec<AnimatedRect>,
    rng: Rng,
    last_time: f64,
    frame: u64,
    /// Uploaded image IDs (populated on first render).
    image_ids: Vec<ImageId>,
    /// Tracks what opacity mode images were generated with.
    images_were_opaque: bool,
}

impl RectScene {
    /// Create a new rectangle benchmark scene with default parameters.
    pub fn new() -> Self {
        Self {
            num_rects: 500,
            speed: 5.0,
            paint_mode: 0,
            rect_size: 50.0,
            rotated: false,
            image_filter: 0,
            image_opaque: false,
            dynamic_gradient: false,
            gradient_shape: 0,
            rects: Vec::new(),
            rng: Rng::new(0xDEAD_BEEF),
            last_time: 0.0,
            frame: 0,
            image_ids: Vec::new(),
            images_were_opaque: false,
        }
    }

    /// Grow or shrink the rect list to match `self.num_rects`, preserving existing rects.
    fn resize_rects(&mut self, w: f64, h: f64) {
        if self.rects.len() < self.num_rects {
            self.rects.reserve(self.num_rects - self.rects.len());
            while self.rects.len() < self.num_rects {
                let r = random_rect(&mut self.rng, w, h);
                self.rects.push(r);
            }
        } else {
            self.rects.truncate(self.num_rects);
        }
    }

    /// Upload patterned images to the renderer (once).
    ///
    /// Each image gets a concentric-ring pattern with a unique frequency and
    /// color palette — cheap to compute but produces visible moiré when scaled,
    /// making the difference between nearest-neighbor and bilinear obvious.
    fn ensure_images(&mut self, scene: &mut DrawContext, backend: &mut Backend) {
        if !self.image_ids.is_empty() && self.images_were_opaque == self.image_opaque {
            return;
        }
        self.image_ids.clear();
        self.images_were_opaque = self.image_opaque;
        let mut rng = Rng::new(0xCAFE_BABE);
        let s = IMAGE_SIZE as f64;
        let cx = s / 2.0;
        let cy = s / 2.0;

        for _ in 0..NUM_IMAGES {
            // Random palette: two colours that alternate in the ring pattern.
            let c1 = rng.color(255);
            let c2 = rng.color(255);
            let [r1, g1, b1, _] = c1.to_rgba8().to_u8_array();
            let [r2, g2, b2, _] = c2.to_rgba8().to_u8_array();
            // Frequency: how many rings fit in the image (3..8).
            let freq = rng.f64() * 5.0 + 3.0;
            let max_dist = (cx * cx + cy * cy).sqrt();

            let mut pixels = vec![
                PremulRgba8 {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 0
                };
                s as usize * s as usize
            ];

            for y in 0..IMAGE_SIZE {
                for x in 0..IMAGE_SIZE {
                    let dx = x as f64 - cx;
                    let dy = y as f64 - cy;
                    let dist = (dx * dx + dy * dy).sqrt();
                    // sin² gives smooth concentric rings.
                    let t = (dist * freq * std::f64::consts::TAU / s).sin();
                    let t = (t * t) as f32; // 0..1

                    let alpha_f = if self.image_opaque {
                        1.0
                    } else {
                        // Alpha fades from fully opaque at center to ~30% at edges.
                        1.0 - 0.7 * (dist / max_dist) as f32
                    };
                    let a = (alpha_f * 255.0) as u8;

                    // Premultiply RGB by alpha.
                    let lerp_premul = |c1: u8, c2: u8| -> u8 {
                        let c = c1 as f32 + (c2 as f32 - c1 as f32) * t;
                        (c * alpha_f) as u8
                    };
                    let idx = y as usize * IMAGE_SIZE as usize + x as usize;
                    pixels[idx] = PremulRgba8 {
                        r: lerp_premul(r1, r2),
                        g: lerp_premul(g1, g2),
                        b: lerp_premul(b1, b2),
                        a,
                    };
                }
            }

            let pixmap =
                Pixmap::from_parts_with_opacity(pixels, IMAGE_SIZE, IMAGE_SIZE, !self.image_opaque);
            let id = backend.upload_image(scene, pixmap);
            self.image_ids.push(id);
        }
    }
}

fn random_rect(rng: &mut Rng, w: f64, h: f64) -> AnimatedRect {
    AnimatedRect {
        x: rng.f64() * w,
        y: rng.f64() * h,
        vx: (rng.f64() - 0.5) * 200.0,
        vy: (rng.f64() - 0.5) * 200.0,
        color: rng.color(200),
        color2: rng.color(200),
        grad_anim: GradientAnim::generate(rng),
        image_idx: (rng.f64() * NUM_IMAGES as f64) as usize % NUM_IMAGES,
        angle: (rng.f64() - 0.5) * std::f64::consts::TAU,
    }
}

impl BenchScene for RectScene {
    fn name(&self) -> &str {
        "Rectangles"
    }

    fn params(&self) -> Vec<Param> {
        vec![
            Param {
                name: "num_rects",
                label: "Rectangles",
                kind: ParamKind::Slider {
                    min: 1.0,
                    max: 200_000.0,
                    step: 1.0,
                },
                value: self.num_rects as f64,
            },
            Param {
                name: "paint_mode",
                label: "Paint",
                kind: ParamKind::Select(vec![("Solid", 0.0), ("Gradient", 1.0), ("Image", 2.0)]),
                value: self.paint_mode as f64,
            },
            Param {
                name: "rect_size",
                label: "Rect Size",
                kind: ParamKind::Slider {
                    min: 5.0,
                    max: 500.0,
                    step: 1.0,
                },
                value: self.rect_size,
            },
            Param {
                name: "rotated",
                label: "Rotated",
                kind: ParamKind::Select(vec![("Off", 0.0), ("On", 1.0)]),
                value: if self.rotated { 1.0 } else { 0.0 },
            },
            Param {
                name: "gradient_shape",
                label: "Gradient Shape",
                kind: ParamKind::Select(vec![("Linear", 0.0), ("Radial", 1.0), ("Sweep", 2.0)]),
                value: self.gradient_shape as f64,
            },
            Param {
                name: "dynamic_gradient",
                label: "Dynamic Gradient",
                kind: ParamKind::Select(vec![("Off", 0.0), ("On", 1.0)]),
                value: if self.dynamic_gradient { 1.0 } else { 0.0 },
            },
            Param {
                name: "image_filter",
                label: "Image Filter",
                kind: ParamKind::Select(vec![("Nearest", 0.0), ("Bilinear", 1.0)]),
                value: self.image_filter as f64,
            },
            Param {
                name: "image_opaque",
                label: "Image Opaque",
                kind: ParamKind::Select(vec![("No", 0.0), ("Yes", 1.0)]),
                value: if self.image_opaque { 1.0 } else { 0.0 },
            },
        ]
    }

    fn set_param(&mut self, name: &str, value: f64) {
        match name {
            "num_rects" => self.num_rects = value as usize,
            "paint_mode" => self.paint_mode = value as u32,
            "rect_size" => self.rect_size = value,
            "rotated" => self.rotated = value >= 0.5,
            "gradient_shape" => self.gradient_shape = value as u32,
            "dynamic_gradient" => self.dynamic_gradient = value >= 0.5,
            "image_filter" => self.image_filter = value as u32,
            "image_opaque" => self.image_opaque = value >= 0.5,
            _ => {}
        }
    }

    fn render(
        &mut self,
        scene: &mut DrawContext,
        backend: &mut Backend,
        width: u32,
        height: u32,
        time: f64,
        view: Affine,
    ) {
        let w = width as f64;
        let h = height as f64;

        // Ensure rect count matches (preserving existing rects).
        if self.rects.len() != self.num_rects {
            self.resize_rects(w, h);
        }

        // Lazily upload images on first use.
        if self.paint_mode == 2 {
            self.ensure_images(scene, backend);
        }

        let dt = delta_time(&mut self.last_time, time, self.speed);
        let frame = self.frame;
        self.frame += 1;

        let size = self.rect_size;
        let half = size / 2.0;

        scene.set_transform(view);

        for r in &mut self.rects {
            r.x += r.vx * dt;
            r.y += r.vy * dt;
            bounce(&mut r.x, &mut r.vx, w - size);
            bounce(&mut r.y, &mut r.vy, h - size);

            // Apply rotation: translate to center, rotate, translate back.
            if self.rotated {
                let cx = r.x + half;
                let cy = r.y + half;
                scene.set_transform(
                    view * Affine::translate((cx, cy))
                        * Affine::rotate(r.angle)
                        * Affine::translate((-half, -half)),
                );
            }

            let rect = if self.rotated {
                // When rotated, rect is at origin (transform handles position).
                Rect::new(0.0, 0.0, size, size)
            } else {
                Rect::new(r.x, r.y, r.x + size, r.y + size)
            };

            match self.paint_mode {
                0 => {
                    scene.set_paint(r.color);
                }
                1 => {
                    let (gx, gy) = if self.rotated { (0.0, 0.0) } else { (r.x, r.y) };
                    let (c1, c2) = if self.dynamic_gradient {
                        // Every rect gets unique colors every frame → unique cache keys.
                        (
                            r.grad_anim.color1.sample(frame),
                            r.grad_anim.color2.sample(frame),
                        )
                    } else {
                        (r.color, r.color2)
                    };
                    let stops = ColorStops(smallvec![
                        ColorStop {
                            offset: 0.0,
                            color: DynamicColor::from_alpha_color(c1),
                        },
                        ColorStop {
                            offset: 1.0,
                            color: DynamicColor::from_alpha_color(c2),
                        },
                    ]);
                    let cx = gx + size * 0.5;
                    let cy = gy + size * 0.5;
                    let dyn_on = self.dynamic_gradient;
                    let kind = match self.gradient_shape {
                        1 => {
                            // Radial: animate end radius between 20%–80% of half-size.
                            let end_r = if dyn_on {
                                let t = r.grad_anim.radius.sample(frame);
                                (0.5 + t * 0.3) * size as f32 * 0.5
                            } else {
                                size as f32 * 0.5
                            };
                            RadialGradientPosition {
                                start_center: Point::new(cx, cy),
                                start_radius: 0.0,
                                end_center: Point::new(cx, cy),
                                end_radius: end_r,
                            }
                            .into()
                        }
                        2 => {
                            // Sweep: animate end angle around full circle.
                            let end_angle = if dyn_on {
                                let t = r.grad_anim.sweep.sample(frame);
                                // Oscillate between π/2 and 2π.
                                std::f32::consts::FRAC_PI_2
                                    + (1.0 + t)
                                        * 0.5
                                        * (std::f32::consts::TAU - std::f32::consts::FRAC_PI_2)
                            } else {
                                std::f32::consts::TAU
                            };
                            SweepGradientPosition {
                                center: Point::new(cx, cy),
                                start_angle: 0.0,
                                end_angle,
                            }
                            .into()
                        }
                        _ => {
                            // Linear: animate gradient line angle around the center.
                            if dyn_on {
                                let a = r.grad_anim.angle.sample(frame) * std::f32::consts::PI;
                                let dx = (a.cos() as f64) * half;
                                let dy = (a.sin() as f64) * half;
                                LinearGradientPosition {
                                    start: Point::new(cx - dx, cy - dy),
                                    end: Point::new(cx + dx, cy + dy),
                                }
                                .into()
                            } else {
                                LinearGradientPosition {
                                    start: Point::new(gx, gy),
                                    end: Point::new(gx + size, gy + size),
                                }
                                .into()
                            }
                        }
                    };
                    let gradient = Gradient {
                        kind,
                        stops,
                        extend: Extend::Pad,
                        ..Default::default()
                    };
                    scene.set_paint(gradient);
                }
                _ => {
                    // Image paint mode.
                    let id = self.image_ids[r.image_idx];
                    let image = Image {
                        image: ImageSource::opaque_id_with_opacity_hint(id, !self.image_opaque),
                        sampler: ImageSampler {
                            x_extend: Extend::Repeat,
                            y_extend: Extend::Repeat,
                            quality: if self.image_filter == 0 {
                                ImageQuality::Low
                            } else {
                                ImageQuality::Medium
                            },
                            alpha: 1.0,
                        },
                    };
                    // Scale image to fill the rect.
                    let scale = size / IMAGE_SIZE as f64;
                    if self.rotated {
                        scene.set_paint_transform(Affine::scale(scale));
                    } else {
                        scene.set_paint_transform(
                            Affine::translate((r.x, r.y)) * Affine::scale(scale),
                        );
                    }
                    scene.set_paint(image);
                }
            }

            scene.fill_rect(&rect);

            if self.rotated {
                scene.set_transform(view);
            }
            if self.paint_mode == 2 {
                scene.reset_paint_transform();
            }
        }
    }
}
