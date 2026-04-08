//! Animated text benchmark backend.

#![allow(
    clippy::cast_possible_truncation,
    reason = "truncation has no appreciable impact in this benchmark"
)]

use std::sync::Arc;

use super::{BenchScene, Param, ParamId, ParamKind, SceneId, bounce, delta_time};
use crate::backend::Renderer;
use crate::rng::Rng;
use skrifa::MetadataProvider;
use skrifa::raw::FileRef;
use vello_common::kurbo::Affine;
use vello_common::peniko::{Blob, Color, FontData};

const INCONSOLATA: &[u8] = include_bytes!("../../assets/Inconsolata.ttf");

/// Printable ASCII range used for random text generation.
const ASCII_START: u8 = b'!';
const ASCII_END: u8 = b'~';

/// An animated glyph run with position, velocity, color, and pre-resolved glyphs.
#[derive(Debug)]
struct AnimatedText {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    color: Color,
    text: String,
    /// Total advance width of the run in pixels.
    run_width: f32,
}

/// Benchmark scene that renders many animated text runs.
pub struct TextScene {
    num_runs: usize,
    speed: f64,
    font_size: f32,
    /// When true, text colors use alpha 255 instead of 220.
    opaque: bool,
    runs: Vec<AnimatedText>,
    rng: Rng,
    last_time: f64,
    font_data: FontData,
}

impl std::fmt::Debug for TextScene {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextScene")
            .field("num_runs", &self.num_runs)
            .field("font_size", &self.font_size)
            .finish_non_exhaustive()
    }
}

impl TextScene {
    /// Create a new text benchmark scene with default parameters.
    pub fn new() -> Self {
        Self {
            num_runs: 200,
            speed: 5.0,
            font_size: 24.0,
            opaque: false,
            runs: Vec::new(),
            rng: Rng::new(0xBAAD_F00D),
            last_time: 0.0,
            font_data: FontData::new(Blob::new(Arc::new(INCONSOLATA)), 0),
        }
    }

    fn resize_runs(&mut self, w: f64, h: f64) {
        let font_ref = match FileRef::new(INCONSOLATA).unwrap() {
            FileRef::Font(f) => f,
            FileRef::Collection(c) => c.get(0).unwrap(),
        };
        let size = skrifa::instance::Size::new(self.font_size);
        let charmap = font_ref.charmap();
        let glyph_metrics = font_ref.glyph_metrics(size, skrifa::instance::LocationRef::default());
        let alpha = if self.opaque { 255 } else { 220 };

        while self.runs.len() < self.num_runs {
            let run = random_text_run(&mut self.rng, w, h, alpha, &charmap, &glyph_metrics);
            self.runs.push(run);
        }
        self.runs.truncate(self.num_runs);
    }
}

fn random_text_run(
    rng: &mut Rng,
    w: f64,
    h: f64,
    alpha: u8,
    charmap: &skrifa::charmap::Charmap<'_>,
    glyph_metrics: &skrifa::metrics::GlyphMetrics<'_>,
) -> AnimatedText {
    // Random length between 3 and 16 characters.
    let len = 3 + (rng.f64() * 14.0) as usize;

    let mut text = String::with_capacity(len);
    let mut pen_x: f32 = 0.0;

    for _ in 0..len {
        let ch = (ASCII_START + (rng.f64() * (ASCII_END - ASCII_START + 1) as f64) as u8) as char;
        text.push(ch);
        let gid = charmap.map(ch).unwrap_or_default();
        let advance = glyph_metrics.advance_width(gid).unwrap_or_default();
        pen_x += advance;
    }

    let run_width = pen_x;
    // Place the run so it starts within the viewport.
    let max_x = (w - run_width as f64).max(0.0);
    let max_y = h;

    AnimatedText {
        x: rng.f64() * max_x,
        y: rng.f64() * max_y,
        vx: (rng.f64() - 0.5) * 200.0,
        vy: (rng.f64() - 0.5) * 200.0,
        text,
        color: rng.color(alpha),
        run_width,
    }
}

impl BenchScene for TextScene {
    fn scene_id(&self) -> SceneId {
        SceneId::Text
    }

    fn name(&self) -> &str {
        "Text"
    }

    fn params(&self) -> Vec<Param> {
        vec![
            Param {
                id: ParamId::NumRuns,
                label: "Text Runs",
                kind: ParamKind::Slider {
                    min: 1.0,
                    max: 10_000.0,
                    step: 1.0,
                },
                value: self.num_runs as f64,
            },
            Param {
                id: ParamId::FontSize,
                label: "Font Size",
                kind: ParamKind::Slider {
                    min: 8.0,
                    max: 128.0,
                    step: 1.0,
                },
                value: self.font_size as f64,
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
            ParamId::NumRuns => self.num_runs = value as usize,
            ParamId::FontSize => {
                let new_size = value as f32;
                if (new_size - self.font_size).abs() > 0.01 {
                    self.font_size = new_size;
                    self.runs.clear();
                }
            }
            ParamId::Opaque => {
                let new_val = value >= 0.5;
                if new_val != self.opaque {
                    self.opaque = new_val;
                    self.runs.clear();
                }
            }
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

        if self.runs.len() != self.num_runs {
            self.resize_runs(w, h);
        }

        let dt = delta_time(&mut self.last_time, time, self.speed);

        backend.set_transform(view);

        for run in &mut self.runs {
            run.x += run.vx * dt;
            run.y += run.vy * dt;
            bounce(&mut run.x, &mut run.vx, (w - run.run_width as f64).max(0.0));
            bounce(&mut run.y, &mut run.vy, h);

            backend.set_paint(run.color.into());
            backend.draw_text(
                &self.font_data,
                self.font_size,
                true,
                &run.text,
                run.x as f32,
                run.y as f32,
            );
        }
    }
}
