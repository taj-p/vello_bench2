//! Backend abstraction over vello_hybrid, vello_cpu, Pathfinder, and Canvas 2D.

mod canvas2d;
mod cpu;
mod hybrid;
mod pathfinder;

use skrifa::MetadataProvider;
use skrifa::raw::FileRef;
use vello_common::filter_effects::Filter;
use vello_common::glyph::Glyph;
use vello_common::kurbo::{Affine, BezPath, Rect, Stroke};
use vello_common::paint::{ImageSource, PaintType};
use vello_common::peniko::{Fill, FontData};
use web_sys::HtmlCanvasElement;

use crate::capability::CapabilityProfile;
use crate::scenes::{ParamId, SceneId};

pub use vello_common::pixmap::Pixmap;

// TODO: Unify image handling across backends around explicit uploaded image
// handles instead of passing `ImageSource` through the renderer API. That
// should include explicit destruction once scene-cached images are no longer
// needed, so backends can release atlas/registry/storage resources promptly.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Hybrid,
    Cpu,
    Pathfinder,
    Canvas2d,
}

impl BackendKind {
    pub const ALL: [Self; 4] = [Self::Hybrid, Self::Cpu, Self::Pathfinder, Self::Canvas2d];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hybrid => "hybrid",
            Self::Cpu => "cpu",
            Self::Pathfinder => "pathfinder",
            Self::Canvas2d => "canvas2d",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Hybrid => "Vello Hybrid",
            Self::Cpu => "Vello CPU",
            Self::Pathfinder => "Pathfinder",
            Self::Canvas2d => "Canvas 2D",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "hybrid" => Some(Self::Hybrid),
            "cpu" => Some(Self::Cpu),
            "pathfinder" => Some(Self::Pathfinder),
            "canvas2d" => Some(Self::Canvas2d),
            _ => None,
        }
    }

    fn capabilities(self) -> &'static CapabilityProfile {
        match self {
            Self::Hybrid => &hybrid::CAPABILITIES,
            Self::Cpu => &cpu::CAPABILITIES,
            Self::Pathfinder => &pathfinder::CAPABILITIES,
            Self::Canvas2d => &canvas2d::CAPABILITIES,
        }
    }
}

pub fn current_backend_kind() -> BackendKind {
    crate::storage::load_backend_name()
        .as_deref()
        .and_then(BackendKind::from_str)
        .unwrap_or(BackendKind::Hybrid)
}

pub trait Renderer {
    fn supports_encode_timing(&self) -> bool;
    fn set_paint(&mut self, paint: PaintType);
    fn set_transform(&mut self, transform: Affine);
    fn reset_transform(&mut self);
    fn set_stroke(&mut self, stroke: Stroke);
    fn set_paint_transform(&mut self, transform: Affine);
    fn reset_paint_transform(&mut self);
    fn set_fill_rule(&mut self, fill: Fill);
    fn fill_rect(&mut self, rect: &Rect);
    fn fill_path(&mut self, path: &BezPath);
    fn stroke_path(&mut self, path: &BezPath);
    fn push_clip_path(&mut self, path: &BezPath);
    fn push_clip_layer(&mut self, path: &BezPath);
    fn push_filter_effect(&mut self, filter: Filter);
    fn pop_clip_path(&mut self);
    fn pop_layer(&mut self);
    fn draw_text(
        &mut self,
        font: &FontData,
        font_size: f32,
        hint: bool,
        text: &str,
        x: f32,
        y: f32,
    );
    fn draw_image(&mut self, image: ImageSource, rect: &Rect, bilinear: bool);
    fn upload_image(&mut self, pixmap: Pixmap) -> ImageSource;
}

pub fn layout_text_glyphs(
    font: &FontData,
    font_size: f32,
    text: &str,
    x: f32,
    y: f32,
) -> Vec<Glyph> {
    let font_ref = match FileRef::new(font.data.as_ref()).unwrap() {
        FileRef::Font(f) => f,
        FileRef::Collection(c) => c.get(font.index).unwrap(),
    };
    let size = skrifa::instance::Size::new(font_size);
    let charmap = font_ref.charmap();
    let glyph_metrics = font_ref.glyph_metrics(size, skrifa::instance::LocationRef::default());
    let mut pen_x = x;
    let mut glyphs = Vec::with_capacity(text.len());
    for ch in text.chars() {
        let gid = charmap.map(ch).unwrap_or_default();
        glyphs.push(Glyph {
            id: gid.to_u32(),
            x: pen_x,
            y,
        });
        pen_x += glyph_metrics.advance_width(gid).unwrap_or_default();
    }
    glyphs
}

#[derive(Debug, Clone, Copy)]
pub struct BackendCapabilities {
    kind: BackendKind,
}

impl BackendCapabilities {
    pub fn supports_scene(self, scene_id: SceneId) -> bool {
        self.kind.capabilities().supports_scene(scene_id)
    }

    pub fn supports_param(self, scene_id: SceneId, param: ParamId) -> bool {
        self.kind.capabilities().supports_param(scene_id, param)
    }

    pub fn supports_param_value(self, scene_id: SceneId, param: ParamId, value: f64) -> bool {
        self.kind
            .capabilities()
            .supports_param_value(scene_id, param, value)
    }
}

pub fn current_backend_capabilities(kind: BackendKind) -> BackendCapabilities {
    BackendCapabilities { kind }
}

pub struct Backend {
    kind: BackendKind,
    inner: BackendImpl,
}

enum BackendImpl {
    Hybrid(hybrid::BackendImpl),
    Cpu(cpu::BackendImpl),
    Pathfinder(pathfinder::BackendImpl),
    Canvas2d(canvas2d::BackendImpl),
}

impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.inner {
            BackendImpl::Hybrid(inner) => inner.fmt(f),
            BackendImpl::Cpu(inner) => inner.fmt(f),
            BackendImpl::Pathfinder(inner) => inner.fmt(f),
            BackendImpl::Canvas2d(inner) => inner.fmt(f),
        }
    }
}

impl Backend {
    pub fn new(canvas: &HtmlCanvasElement, w: u32, h: u32, kind: BackendKind) -> Self {
        let inner = match kind {
            BackendKind::Hybrid => BackendImpl::Hybrid(hybrid::BackendImpl::new(canvas, w, h)),
            BackendKind::Cpu => BackendImpl::Cpu(cpu::BackendImpl::new(canvas, w, h)),
            BackendKind::Pathfinder => {
                BackendImpl::Pathfinder(pathfinder::BackendImpl::new(canvas, w, h))
            }
            BackendKind::Canvas2d => {
                BackendImpl::Canvas2d(canvas2d::BackendImpl::new(canvas, w, h))
            }
        };
        Self { kind, inner }
    }

    pub fn kind(&self) -> BackendKind {
        self.kind
    }

    pub fn reset(&mut self) {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.reset(),
            BackendImpl::Cpu(inner) => inner.reset(),
            BackendImpl::Pathfinder(inner) => inner.reset(),
            BackendImpl::Canvas2d(inner) => inner.reset(),
        }
    }

    pub fn render_offscreen(&mut self) {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.render_offscreen(),
            BackendImpl::Cpu(inner) => inner.render_offscreen(),
            BackendImpl::Pathfinder(inner) => inner.render_offscreen(),
            BackendImpl::Canvas2d(inner) => inner.render_offscreen(),
        }
    }

    pub fn blit(&mut self) {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.blit(),
            BackendImpl::Cpu(inner) => inner.blit(),
            BackendImpl::Pathfinder(inner) => inner.blit(),
            BackendImpl::Canvas2d(inner) => inner.blit(),
        }
    }

    pub fn is_cpu(&self) -> bool {
        match &self.inner {
            BackendImpl::Hybrid(inner) => inner.is_cpu(),
            BackendImpl::Cpu(inner) => inner.is_cpu(),
            BackendImpl::Pathfinder(inner) => inner.is_cpu(),
            BackendImpl::Canvas2d(inner) => inner.is_cpu(),
        }
    }

    pub fn supports_encode_timing(&self) -> bool {
        match &self.inner {
            BackendImpl::Hybrid(inner) => inner.supports_encode_timing(),
            BackendImpl::Cpu(inner) => inner.supports_encode_timing(),
            BackendImpl::Pathfinder(inner) => inner.supports_encode_timing(),
            BackendImpl::Canvas2d(inner) => inner.supports_encode_timing(),
        }
    }

    pub fn sync(&self) {
        match &self.inner {
            BackendImpl::Hybrid(inner) => inner.sync(),
            BackendImpl::Cpu(inner) => inner.sync(),
            BackendImpl::Pathfinder(inner) => inner.sync(),
            BackendImpl::Canvas2d(inner) => inner.sync(),
        }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.resize(w, h),
            BackendImpl::Cpu(inner) => inner.resize(w, h),
            BackendImpl::Pathfinder(inner) => inner.resize(w, h),
            BackendImpl::Canvas2d(inner) => inner.resize(w, h),
        }
    }
}

impl Renderer for Backend {
    fn supports_encode_timing(&self) -> bool {
        self.supports_encode_timing()
    }

    fn set_paint(&mut self, paint: PaintType) {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.set_paint(paint),
            BackendImpl::Cpu(inner) => inner.set_paint(paint),
            BackendImpl::Pathfinder(inner) => inner.set_paint(paint),
            BackendImpl::Canvas2d(inner) => inner.set_paint(paint),
        }
    }

    fn set_transform(&mut self, transform: Affine) {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.set_transform(transform),
            BackendImpl::Cpu(inner) => inner.set_transform(transform),
            BackendImpl::Pathfinder(inner) => inner.set_transform(transform),
            BackendImpl::Canvas2d(inner) => inner.set_transform(transform),
        }
    }

    fn reset_transform(&mut self) {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.reset_transform(),
            BackendImpl::Cpu(inner) => inner.reset_transform(),
            BackendImpl::Pathfinder(inner) => inner.reset_transform(),
            BackendImpl::Canvas2d(inner) => inner.reset_transform(),
        }
    }

    fn set_stroke(&mut self, stroke: Stroke) {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.set_stroke(stroke),
            BackendImpl::Cpu(inner) => inner.set_stroke(stroke),
            BackendImpl::Pathfinder(inner) => inner.set_stroke(stroke),
            BackendImpl::Canvas2d(inner) => inner.set_stroke(stroke),
        }
    }

    fn set_paint_transform(&mut self, transform: Affine) {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.set_paint_transform(transform),
            BackendImpl::Cpu(inner) => inner.set_paint_transform(transform),
            BackendImpl::Pathfinder(inner) => inner.set_paint_transform(transform),
            BackendImpl::Canvas2d(inner) => inner.set_paint_transform(transform),
        }
    }

    fn reset_paint_transform(&mut self) {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.reset_paint_transform(),
            BackendImpl::Cpu(inner) => inner.reset_paint_transform(),
            BackendImpl::Pathfinder(inner) => inner.reset_paint_transform(),
            BackendImpl::Canvas2d(inner) => inner.reset_paint_transform(),
        }
    }

    fn set_fill_rule(&mut self, fill: Fill) {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.set_fill_rule(fill),
            BackendImpl::Cpu(inner) => inner.set_fill_rule(fill),
            BackendImpl::Pathfinder(inner) => inner.set_fill_rule(fill),
            BackendImpl::Canvas2d(inner) => inner.set_fill_rule(fill),
        }
    }

    fn fill_rect(&mut self, rect: &Rect) {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.fill_rect(rect),
            BackendImpl::Cpu(inner) => inner.fill_rect(rect),
            BackendImpl::Pathfinder(inner) => inner.fill_rect(rect),
            BackendImpl::Canvas2d(inner) => inner.fill_rect(rect),
        }
    }

    fn fill_path(&mut self, path: &BezPath) {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.fill_path(path),
            BackendImpl::Cpu(inner) => inner.fill_path(path),
            BackendImpl::Pathfinder(inner) => inner.fill_path(path),
            BackendImpl::Canvas2d(inner) => inner.fill_path(path),
        }
    }

    fn stroke_path(&mut self, path: &BezPath) {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.stroke_path(path),
            BackendImpl::Cpu(inner) => inner.stroke_path(path),
            BackendImpl::Pathfinder(inner) => inner.stroke_path(path),
            BackendImpl::Canvas2d(inner) => inner.stroke_path(path),
        }
    }

    fn push_clip_path(&mut self, path: &BezPath) {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.push_clip_path(path),
            BackendImpl::Cpu(inner) => inner.push_clip_path(path),
            BackendImpl::Pathfinder(inner) => inner.push_clip_path(path),
            BackendImpl::Canvas2d(inner) => inner.push_clip_path(path),
        }
    }

    fn push_clip_layer(&mut self, path: &BezPath) {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.push_clip_layer(path),
            BackendImpl::Cpu(inner) => inner.push_clip_layer(path),
            BackendImpl::Pathfinder(inner) => inner.push_clip_layer(path),
            BackendImpl::Canvas2d(inner) => inner.push_clip_layer(path),
        }
    }

    fn push_filter_effect(&mut self, filter: Filter) {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.push_filter_layer(filter),
            BackendImpl::Cpu(inner) => inner.push_filter_layer(filter),
            BackendImpl::Pathfinder(inner) => inner.push_filter_layer(filter),
            BackendImpl::Canvas2d(inner) => inner.push_filter_layer(filter),
        }
    }

    fn pop_clip_path(&mut self) {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.pop_clip_path(),
            BackendImpl::Cpu(inner) => inner.pop_clip_path(),
            BackendImpl::Pathfinder(inner) => inner.pop_clip_path(),
            BackendImpl::Canvas2d(inner) => inner.pop_clip_path(),
        }
    }

    fn pop_layer(&mut self) {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.pop_layer(),
            BackendImpl::Cpu(inner) => inner.pop_layer(),
            BackendImpl::Pathfinder(inner) => inner.pop_layer(),
            BackendImpl::Canvas2d(inner) => inner.pop_layer(),
        }
    }

    fn draw_text(
        &mut self,
        font: &FontData,
        font_size: f32,
        hint: bool,
        text: &str,
        x: f32,
        y: f32,
    ) {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.draw_text(font, font_size, hint, text, x, y),
            BackendImpl::Cpu(inner) => inner.draw_text(font, font_size, hint, text, x, y),
            BackendImpl::Pathfinder(inner) => inner.draw_text(font, font_size, hint, text, x, y),
            BackendImpl::Canvas2d(inner) => inner.draw_text(font, font_size, hint, text, x, y),
        }
    }

    fn draw_image(&mut self, image: ImageSource, rect: &Rect, bilinear: bool) {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.draw_image(image, rect, bilinear),
            BackendImpl::Cpu(inner) => inner.draw_image(image, rect, bilinear),
            BackendImpl::Pathfinder(inner) => inner.draw_image(image, rect, bilinear),
            BackendImpl::Canvas2d(inner) => inner.draw_image(image, rect, bilinear),
        }
    }

    fn upload_image(&mut self, pixmap: Pixmap) -> ImageSource {
        match &mut self.inner {
            BackendImpl::Hybrid(inner) => inner.upload_image(pixmap),
            BackendImpl::Cpu(inner) => inner.upload_image(pixmap),
            BackendImpl::Pathfinder(inner) => inner.upload_image(pixmap),
            BackendImpl::Canvas2d(inner) => inner.upload_image(pixmap),
        }
    }
}
