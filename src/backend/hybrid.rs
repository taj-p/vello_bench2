use vello_common::filter_effects::Filter;
use vello_common::glyph::Glyph;
use vello_common::kurbo::{Affine, BezPath, Rect, Stroke};
use vello_common::paint::{ImageSource, PaintType};
use vello_common::peniko::{Fill, FontData};
use vello_common::pixmap::Pixmap;
use web_sys::HtmlCanvasElement;

use crate::backend::{Backend, BackendKind, layout_text_glyphs};
use crate::capability::CapabilityProfile;

pub(crate) const CAPABILITIES: CapabilityProfile = CapabilityProfile::all();

pub struct BackendImpl {
    ctx: vello_hybrid::Scene,
    renderer: vello_hybrid::WebGlRenderer,
}

impl std::fmt::Debug for BackendImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backend(hybrid)").finish()
    }
}

impl BackendImpl {
    pub fn new(canvas: &HtmlCanvasElement, w: u32, h: u32) -> Self {
        Self {
            ctx: vello_hybrid::Scene::new(w as u16, h as u16),
            renderer: vello_hybrid::WebGlRenderer::new(canvas),
        }
    }

    fn draw_glyphs(&mut self, font: &FontData, font_size: f32, hint: bool, glyphs: &[Glyph]) {
        self.ctx
            .glyph_run(font)
            .font_size(font_size)
            .hint(hint)
            .fill_glyphs(glyphs.iter().copied());
    }
}

impl Backend for BackendImpl {
    fn kind(&self) -> BackendKind {
        BackendKind::Hybrid
    }

    fn reset(&mut self) {
        self.ctx.reset();
    }

    fn render_offscreen(&mut self) {
        let rs = vello_hybrid::RenderSize {
            width: self.ctx.width() as u32,
            height: self.ctx.height() as u32,
        };
        self.renderer.render(&mut self.ctx, &rs).unwrap();
    }

    fn blit(&mut self) {}

    fn is_cpu(&self) -> bool {
        false
    }

    fn supports_encode_timing(&self) -> bool {
        true
    }

    fn sync(&self) {
        crate::gpu_sync(&self.renderer);
    }

    fn resize(&mut self, w: u32, h: u32) {
        self.ctx = vello_hybrid::Scene::new(w as u16, h as u16);
    }

    fn set_paint(&mut self, paint: PaintType) {
        self.ctx.set_paint(paint);
    }

    fn set_transform(&mut self, transform: Affine) {
        self.ctx.set_transform(transform);
    }

    fn reset_transform(&mut self) {
        self.ctx.reset_transform();
    }

    fn set_stroke(&mut self, stroke: Stroke) {
        self.ctx.set_stroke(stroke);
    }

    fn set_paint_transform(&mut self, transform: Affine) {
        self.ctx.set_paint_transform(transform);
    }

    fn reset_paint_transform(&mut self) {
        self.ctx.reset_paint_transform();
    }

    fn set_fill_rule(&mut self, fill: Fill) {
        self.ctx.set_fill_rule(fill);
    }

    fn fill_rect(&mut self, rect: &Rect) {
        self.ctx.fill_rect(rect);
    }

    fn fill_path(&mut self, path: &BezPath) {
        self.ctx.fill_path(path);
    }

    fn stroke_path(&mut self, path: &BezPath) {
        self.ctx.stroke_path(path);
    }

    fn push_clip_path(&mut self, path: &BezPath) {
        self.ctx.push_clip_path(path);
    }

    fn push_clip_layer(&mut self, path: &BezPath) {
        self.ctx.push_clip_layer(path);
    }

    fn set_filter_effect(&mut self, filter: Filter) {
        self.ctx.push_filter_layer(filter);
    }

    fn pop_clip_path(&mut self) {
        self.ctx.pop_clip_path();
    }

    fn pop_layer(&mut self) {
        self.ctx.pop_layer();
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
        let glyphs = layout_text_glyphs(font, font_size, text, x, y);
        self.draw_glyphs(font, font_size, hint, &glyphs);
    }

    fn draw_image(&mut self, _image: ImageSource, _rect: &Rect, _bilinear: bool) {}

    fn upload_image(&mut self, pixmap: Pixmap) -> ImageSource {
        let id = self.renderer.upload_image(&pixmap);
        ImageSource::opaque_id_with_opacity_hint(id, pixmap.may_have_opacities())
    }
}
