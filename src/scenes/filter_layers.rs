//! Animated rectangles wrapped in filter layers.

#![allow(
    clippy::cast_possible_truncation,
    reason = "truncation has no appreciable impact in this benchmark"
)]

use super::{BenchScene, Param, ParamId, ParamKind, SceneId, bounce, delta_time};
use crate::backend::Renderer;
use crate::rng::Rng;
use vello_common::color::AlphaColor;
use vello_common::filter_effects::{EdgeMode, Filter, FilterPrimitive};
use vello_common::kurbo::{Affine, Rect};
use vello_common::peniko::Color;

#[derive(Debug)]
struct AnimatedRect {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    color: Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilterKind {
    DropShadow,
    Blur,
}

/// Scene that renders moving rectangles inside filter layers.
#[derive(Debug)]
pub struct FilterLayersScene {
    num_rects: usize,
    rect_size: f64,
    speed: f64,
    filter_kind: FilterKind,
    blur_std_deviation: f32,
    shadow_dx: f32,
    shadow_dy: f32,
    shadow_alpha: u8,
    rects: Vec<AnimatedRect>,
    rng: Rng,
    last_time: f64,
}

impl FilterLayersScene {
    pub fn new() -> Self {
        Self {
            num_rects: 1,
            rect_size: 400.0,
            speed: 5.0,
            filter_kind: FilterKind::Blur,
            blur_std_deviation: 24.0,
            shadow_dx: 24.0,
            shadow_dy: 24.0,
            shadow_alpha: 160,
            rects: Vec::new(),
            rng: Rng::new(0xF17E_AA55),
            last_time: 0.0,
        }
    }

    fn resize_rects(&mut self, w: f64, h: f64) {
        if self.rects.len() < self.num_rects {
            self.rects.reserve(self.num_rects - self.rects.len());
            while self.rects.len() < self.num_rects {
                self.rects.push(random_rect(&mut self.rng, w, h));
            }
        } else {
            self.rects.truncate(self.num_rects);
        }
    }
}

fn make_filter(
    filter_kind: FilterKind,
    blur_std_deviation: f32,
    shadow_dx: f32,
    shadow_dy: f32,
    shadow_alpha: u8,
    color: Color,
) -> Filter {
    match filter_kind {
        FilterKind::DropShadow => Filter::from_primitive(FilterPrimitive::DropShadow {
            dx: shadow_dx,
            dy: shadow_dy,
            std_deviation: blur_std_deviation,
            color: {
                let [r, g, b, _] = color.to_rgba8().to_u8_array();
                AlphaColor::from_rgba8(r, g, b, shadow_alpha)
            },
            edge_mode: EdgeMode::None,
        }),
        FilterKind::Blur => Filter::from_primitive(FilterPrimitive::GaussianBlur {
            std_deviation: blur_std_deviation,
            edge_mode: EdgeMode::None,
        }),
    }
}

fn random_rect(rng: &mut Rng, w: f64, h: f64) -> AnimatedRect {
    AnimatedRect {
        x: rng.f64() * w,
        y: rng.f64() * h,
        vx: (rng.f64() - 0.5) * 200.0,
        vy: (rng.f64() - 0.5) * 200.0,
        color: rng.color(220),
    }
}

impl BenchScene for FilterLayersScene {
    fn scene_id(&self) -> SceneId {
        SceneId::FilterLayers
    }

    fn name(&self) -> &str {
        "Filter Layers"
    }

    fn params(&self) -> Vec<Param> {
        vec![
            Param {
                id: ParamId::NumRects,
                label: "Rectangles",
                kind: ParamKind::Slider {
                    min: 1.0,
                    max: 50_000.0,
                    step: 1.0,
                },
                value: self.num_rects as f64,
            },
            Param {
                id: ParamId::RectSize,
                label: "Rect Size",
                kind: ParamKind::Slider {
                    min: 10.0,
                    max: 800.0,
                    step: 1.0,
                },
                value: self.rect_size,
            },
            Param {
                id: ParamId::FilterKind,
                label: "Filter",
                kind: ParamKind::Select(vec![("Drop Shadow", 0.0), ("Blur", 1.0)]),
                value: if self.filter_kind == FilterKind::Blur {
                    1.0
                } else {
                    0.0
                },
            },
            Param {
                id: ParamId::BlurStdDeviation,
                label: "Blur Sigma",
                kind: ParamKind::Slider {
                    min: 0.0,
                    max: 64.0,
                    step: 1.0,
                },
                value: self.blur_std_deviation as f64,
            },
            Param {
                id: ParamId::ShadowDx,
                label: "Shadow DX",
                kind: ParamKind::Slider {
                    min: -128.0,
                    max: 128.0,
                    step: 1.0,
                },
                value: self.shadow_dx as f64,
            },
            Param {
                id: ParamId::ShadowDy,
                label: "Shadow DY",
                kind: ParamKind::Slider {
                    min: -128.0,
                    max: 128.0,
                    step: 1.0,
                },
                value: self.shadow_dy as f64,
            },
            Param {
                id: ParamId::ShadowAlpha,
                label: "Shadow Alpha",
                kind: ParamKind::Slider {
                    min: 0.0,
                    max: 255.0,
                    step: 1.0,
                },
                value: self.shadow_alpha as f64,
            },
        ]
    }

    fn set_param(&mut self, param: ParamId, value: f64) {
        match param {
            ParamId::NumRects => self.num_rects = value as usize,
            ParamId::RectSize => self.rect_size = value,
            ParamId::Speed => self.speed = value,
            ParamId::FilterKind => {
                self.filter_kind = if value >= 0.5 {
                    FilterKind::Blur
                } else {
                    FilterKind::DropShadow
                };
            }
            ParamId::BlurStdDeviation => self.blur_std_deviation = value as f32,
            ParamId::ShadowDx => self.shadow_dx = value as f32,
            ParamId::ShadowDy => self.shadow_dy = value as f32,
            ParamId::ShadowAlpha => self.shadow_alpha = value as u8,
            _ => {}
        }
    }

    fn render(
        &mut self,
        backend: &mut dyn Renderer,
        width: u32,
        height: u32,
        time: f64,
        view: Affine,
    ) {
        let w = width as f64;
        let h = height as f64;

        if self.rects.len() != self.num_rects {
            self.resize_rects(w, h);
        }

        let dt = delta_time(&mut self.last_time, time, self.speed);
        let size = self.rect_size;
        let max_x = (w - size).max(0.0);
        let max_y = (h - size).max(0.0);
        let filter_kind = self.filter_kind;
        let blur_std_deviation = self.blur_std_deviation;
        let shadow_dx = self.shadow_dx;
        let shadow_dy = self.shadow_dy;
        let shadow_alpha = self.shadow_alpha;
        backend.set_transform(view);

        for rect_state in &mut self.rects {
            rect_state.x += rect_state.vx * dt;
            rect_state.y += rect_state.vy * dt;
            bounce(&mut rect_state.x, &mut rect_state.vx, max_x);
            bounce(&mut rect_state.y, &mut rect_state.vy, max_y);

            let rect = Rect::new(
                rect_state.x,
                rect_state.y,
                rect_state.x + size,
                rect_state.y + size,
            );
            backend.set_filter_effect(make_filter(
                filter_kind,
                blur_std_deviation,
                shadow_dx,
                shadow_dy,
                shadow_alpha,
                rect_state.color,
            ));
            backend.set_paint(rect_state.color.into());
            backend.fill_rect(&rect);
            backend.pop_layer();
        }
    }
}
