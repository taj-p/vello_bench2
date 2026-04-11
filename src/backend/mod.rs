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

pub trait Backend {
    fn kind(&self) -> BackendKind;
    fn reset(&mut self);
    fn render_offscreen(&mut self);
    fn blit(&mut self);
    fn is_cpu(&self) -> bool;
    fn supports_encode_timing(&self) -> bool;
    fn sync(&self);
    fn resize(&mut self, w: u32, h: u32);
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
    fn set_filter_effect(&mut self, filter: Filter);
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

pub fn new_backend(
    canvas: &HtmlCanvasElement,
    w: u32,
    h: u32,
    kind: BackendKind,
) -> Box<dyn Backend> {
    match kind {
        BackendKind::Hybrid => Box::new(hybrid::BackendImpl::new(canvas, w, h)),
        BackendKind::Cpu => Box::new(cpu::BackendImpl::new(canvas, w, h)),
        BackendKind::Pathfinder => Box::new(pathfinder::BackendImpl::new(canvas, w, h)),
        BackendKind::Canvas2d => Box::new(canvas2d::BackendImpl::new(canvas, w, h)),
    }
}
