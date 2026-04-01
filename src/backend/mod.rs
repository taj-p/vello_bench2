//! Backend abstraction over vello_hybrid, vello_cpu, and Pathfinder.

use vello_common::filter_effects::Filter;
use vello_common::glyph::Glyph;
use vello_common::kurbo::{Affine, BezPath, Rect, Stroke};
use vello_common::paint::{ImageSource, PaintType};
use vello_common::peniko::{Fill, FontData};
use web_sys::HtmlCanvasElement;

use crate::scenes::{ParamId, SceneId};

#[cfg(feature = "cpu")]
mod cpu;
#[cfg(all(not(feature = "cpu"), not(feature = "pathfinder")))]
mod hybrid;
#[cfg(all(not(feature = "cpu"), feature = "pathfinder"))]
mod pathfinder;

#[cfg(feature = "cpu")]
use cpu as selected;
#[cfg(all(not(feature = "cpu"), not(feature = "pathfinder")))]
use hybrid as selected;
#[cfg(all(not(feature = "cpu"), feature = "pathfinder"))]
use pathfinder as selected;

use selected::BackendImpl;
pub use selected::Pixmap;

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
    fn push_filter_layer(&mut self, filter: Filter);
    fn pop_clip_path(&mut self);
    fn pop_layer(&mut self);
    fn fill_glyphs(&mut self, font: &FontData, font_size: f32, hint: bool, glyphs: &[Glyph]);
    fn draw_image(&mut self, image: ImageSource, rect: &Rect, bilinear: bool);
    fn upload_image(&mut self, pixmap: Pixmap) -> ImageSource;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BackendCapabilities;

impl BackendCapabilities {
    pub fn supports_scene(self, scene_id: SceneId) -> bool {
        selected::supports_scene(scene_id)
    }

    pub fn supports_param(self, scene_id: SceneId, param: ParamId) -> bool {
        selected::supports_param(scene_id, param)
    }
}

pub fn current_backend_capabilities() -> BackendCapabilities {
    BackendCapabilities
}

pub struct Backend {
    inner: BackendImpl,
}

impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(f)
    }
}

impl Backend {
    pub fn new(canvas: &HtmlCanvasElement, w: u32, h: u32) -> Self {
        Self {
            inner: BackendImpl::new(canvas, w, h),
        }
    }

    pub fn reset(&mut self) {
        self.inner.reset();
    }

    pub fn render_offscreen(&mut self) {
        self.inner.render_offscreen();
    }

    pub fn blit(&mut self) {
        self.inner.blit();
    }

    pub fn is_cpu(&self) -> bool {
        self.inner.is_cpu()
    }

    pub fn supports_encode_timing(&self) -> bool {
        self.inner.supports_encode_timing()
    }

    pub fn sync(&self) {
        self.inner.sync();
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.inner.resize(w, h);
    }
}

impl Renderer for Backend {
    fn supports_encode_timing(&self) -> bool {
        self.inner.supports_encode_timing()
    }

    fn set_paint(&mut self, paint: PaintType) {
        self.inner.set_paint(paint);
    }

    fn set_transform(&mut self, transform: Affine) {
        self.inner.set_transform(transform);
    }

    fn reset_transform(&mut self) {
        self.inner.reset_transform();
    }

    fn set_stroke(&mut self, stroke: Stroke) {
        self.inner.set_stroke(stroke);
    }

    fn set_paint_transform(&mut self, transform: Affine) {
        self.inner.set_paint_transform(transform);
    }

    fn reset_paint_transform(&mut self) {
        self.inner.reset_paint_transform();
    }

    fn set_fill_rule(&mut self, fill: Fill) {
        self.inner.set_fill_rule(fill);
    }

    fn fill_rect(&mut self, rect: &Rect) {
        self.inner.fill_rect(rect);
    }

    fn fill_path(&mut self, path: &BezPath) {
        self.inner.fill_path(path);
    }

    fn stroke_path(&mut self, path: &BezPath) {
        self.inner.stroke_path(path);
    }

    fn push_clip_path(&mut self, path: &BezPath) {
        self.inner.push_clip_path(path);
    }

    fn push_clip_layer(&mut self, path: &BezPath) {
        self.inner.push_clip_layer(path);
    }

    fn push_filter_layer(&mut self, filter: Filter) {
        self.inner.push_filter_layer(filter);
    }

    fn pop_clip_path(&mut self) {
        self.inner.pop_clip_path();
    }

    fn pop_layer(&mut self) {
        self.inner.pop_layer();
    }

    fn fill_glyphs(&mut self, font: &FontData, font_size: f32, hint: bool, glyphs: &[Glyph]) {
        self.inner.fill_glyphs(font, font_size, hint, glyphs);
    }

    fn draw_image(&mut self, image: ImageSource, rect: &Rect, bilinear: bool) {
        self.inner.draw_image(image, rect, bilinear);
    }

    fn upload_image(&mut self, pixmap: Pixmap) -> ImageSource {
        self.inner.upload_image(pixmap)
    }
}
