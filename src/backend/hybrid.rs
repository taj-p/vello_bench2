use vello_common::filter_effects::Filter;
use vello_common::glyph::Glyph;
use vello_common::kurbo::{Affine, BezPath, Rect, Stroke};
use vello_common::paint::{ImageSource, PaintType};
use vello_common::peniko::{Fill, FontData};
use vello_common::pixmap::Pixmap;
use web_sys::HtmlCanvasElement;

use crate::backend::layout_text_glyphs;
use crate::scenes::{ParamId, SceneId};

pub fn supports_scene(_scene_id: SceneId) -> bool {
    true
}

pub fn supports_param(_scene_id: SceneId, _param: ParamId) -> bool {
    true
}

pub fn supports_param_value(_scene_id: SceneId, _param: ParamId, _value: f64) -> bool {
    true
}

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

    pub fn reset(&mut self) {
        self.ctx.reset();
    }

    pub fn render_offscreen(&mut self) {
        let rs = vello_hybrid::RenderSize {
            width: self.ctx.width() as u32,
            height: self.ctx.height() as u32,
        };
        self.renderer.render(&mut self.ctx, &rs).unwrap();
    }

    pub fn blit(&mut self) {}

    pub fn is_cpu(&self) -> bool {
        false
    }

    pub fn supports_encode_timing(&self) -> bool {
        true
    }

    pub fn sync(&self) {
        crate::gpu_sync(&self.renderer);
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.ctx = vello_hybrid::Scene::new(w as u16, h as u16);
    }

    pub fn upload_image(&mut self, pixmap: Pixmap) -> ImageSource {
        let id = self.renderer.upload_image(&pixmap);
        ImageSource::opaque_id_with_opacity_hint(id, pixmap.may_have_opacities())
    }

    pub fn set_paint(&mut self, paint: PaintType) {
        self.ctx.set_paint(paint);
    }

    pub fn set_transform(&mut self, transform: Affine) {
        self.ctx.set_transform(transform);
    }

    pub fn reset_transform(&mut self) {
        self.ctx.reset_transform();
    }

    pub fn set_stroke(&mut self, stroke: Stroke) {
        self.ctx.set_stroke(stroke);
    }

    pub fn set_paint_transform(&mut self, transform: Affine) {
        self.ctx.set_paint_transform(transform);
    }

    pub fn reset_paint_transform(&mut self) {
        self.ctx.reset_paint_transform();
    }

    pub fn set_fill_rule(&mut self, fill: Fill) {
        self.ctx.set_fill_rule(fill);
    }

    pub fn fill_rect(&mut self, rect: &Rect) {
        self.ctx.fill_rect(rect);
    }

    pub fn fill_path(&mut self, path: &BezPath) {
        self.ctx.fill_path(path);
    }

    pub fn stroke_path(&mut self, path: &BezPath) {
        self.ctx.stroke_path(path);
    }

    pub fn push_clip_path(&mut self, path: &BezPath) {
        self.ctx.push_clip_path(path);
    }

    pub fn push_clip_layer(&mut self, path: &BezPath) {
        self.ctx.push_clip_layer(path);
    }

    pub fn push_filter_layer(&mut self, filter: Filter) {
        self.ctx.push_filter_layer(filter);
    }

    pub fn pop_clip_path(&mut self) {
        self.ctx.pop_clip_path();
    }

    pub fn pop_layer(&mut self) {
        self.ctx.pop_layer();
    }

    pub fn fill_glyphs(&mut self, font: &FontData, font_size: f32, hint: bool, glyphs: &[Glyph]) {
        self.ctx
            .glyph_run(font)
            .font_size(font_size)
            .hint(hint)
            .fill_glyphs(glyphs.iter().copied());
    }

    pub fn draw_text(
        &mut self,
        font: &FontData,
        font_size: f32,
        hint: bool,
        text: &str,
        x: f32,
        y: f32,
    ) {
        let glyphs = layout_text_glyphs(font, font_size, text, x, y);
        self.fill_glyphs(font, font_size, hint, &glyphs);
    }

    pub fn draw_image(&mut self, _image: ImageSource, _rect: &Rect, _bilinear: bool) {}
}
