use js_sys::{Function, Reflect};
use vello_common::filter::PreparedFilter;
use vello_common::filter_effects::Filter;
use vello_common::glyph::Glyph;
use vello_common::kurbo::{Affine, BezPath, PathEl, Rect, Stroke};
use vello_common::paint::{ImageId, ImageSource, PaintType};
use vello_common::peniko::color::Srgb;
use vello_common::peniko::{
    Fill, FontData, Gradient, GradientKind, ImageQuality, LinearGradientPosition,
    RadialGradientPosition, SweepGradientPosition,
};
use vello_common::pixmap::Pixmap;
use wasm_bindgen::{Clamped, JsCast, JsValue};
use web_sys::{
    CanvasGradient, CanvasRenderingContext2d, CanvasWindingRule, HtmlCanvasElement, ImageData,
    window,
};

use crate::scenes::{ParamId, SceneId};

pub fn supports_scene(scene_id: SceneId) -> bool {
    matches!(
        scene_id,
        SceneId::Rect
            | SceneId::Strokes
            | SceneId::Polyline
            | SceneId::Svg
            | SceneId::Clip
            | SceneId::Text
            | SceneId::FilterLayers
    )
}

pub fn supports_param(scene_id: SceneId, param: ParamId) -> bool {
    matches!(
        (scene_id, param),
        (SceneId::Rect, ParamId::NumRects)
            | (SceneId::Rect, ParamId::PaintMode)
            | (SceneId::Rect, ParamId::RectSize)
            | (SceneId::Rect, ParamId::Rotated)
            | (SceneId::Rect, ParamId::ImageFilter)
            | (SceneId::Rect, ParamId::ImageOpaque)
            | (SceneId::Rect, ParamId::UseDrawImage)
            | (SceneId::Rect, ParamId::GradientShape)
            | (SceneId::Rect, ParamId::DynamicGradient)
            | (SceneId::Strokes, ParamId::NumStrokes)
            | (SceneId::Strokes, ParamId::CurveType)
            | (SceneId::Strokes, ParamId::Segments)
            | (SceneId::Strokes, ParamId::StrokeWidth)
            | (SceneId::Strokes, ParamId::Cap)
            | (SceneId::Polyline, ParamId::NumVertices)
            | (SceneId::Svg, ParamId::SvgAsset)
            | (SceneId::Clip, ParamId::NumRects)
            | (SceneId::Clip, ParamId::RectSize)
            | (SceneId::Clip, ParamId::ClipMode)
            | (SceneId::Clip, ParamId::ClipMethod)
            | (SceneId::Text, ParamId::NumRuns)
            | (SceneId::Text, ParamId::FontSize)
            | (SceneId::FilterLayers, ParamId::NumRects)
            | (SceneId::FilterLayers, ParamId::RectSize)
            | (SceneId::FilterLayers, ParamId::FilterKind)
            | (SceneId::FilterLayers, ParamId::Speed)
            | (SceneId::FilterLayers, ParamId::BlurStdDeviation)
            | (SceneId::FilterLayers, ParamId::ShadowDx)
            | (SceneId::FilterLayers, ParamId::ShadowDy)
            | (SceneId::FilterLayers, ParamId::ShadowAlpha)
    )
}

pub fn supports_param_value(_scene_id: SceneId, _param: ParamId, _value: f64) -> bool {
    true
}

pub struct BackendImpl {
    ctx: CanvasRenderingContext2d,
    current_paint: PaintState,
    fill_rule: CanvasWindingRule,
    clip_depth: usize,
    layer_stack: Vec<LayerKind>,
    current_transform: Affine,
    width: f64,
    height: f64,
    uploaded_images: Vec<UploadedImage>,
}

#[derive(Clone)]
enum PaintState {
    Solid([f32; 4]),
    Gradient(Gradient),
    Image(ImagePaint),
}

#[derive(Clone)]
struct ImagePaint {
    image: ImageSource,
    quality: ImageQuality,
    alpha: f32,
}

struct UploadedImage {
    canvas: HtmlCanvasElement,
    width: f64,
    height: f64,
}

#[derive(Clone, Copy)]
enum LayerKind {
    Clip,
    Filter,
}

impl std::fmt::Debug for BackendImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backend(canvas2d)").finish()
    }
}

impl BackendImpl {
    pub fn new(canvas: &HtmlCanvasElement, w: u32, h: u32) -> Self {
        let ctx: CanvasRenderingContext2d = canvas
            .get_context("2d")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        let mut out = Self {
            ctx,
            current_paint: PaintState::Solid([1.0, 1.0, 1.0, 1.0]),
            fill_rule: CanvasWindingRule::Nonzero,
            clip_depth: 0,
            layer_stack: Vec::new(),
            current_transform: Affine::IDENTITY,
            width: w as f64,
            height: h as f64,
            uploaded_images: Vec::new(),
        };
        out.reset();
        out
    }

    pub fn reset(&mut self) {
        while self.clip_depth > 0 {
            self.ctx.restore();
            self.clip_depth -= 1;
        }
        while self.layer_stack.pop().is_some() {
            self.ctx.restore();
        }
        self.ctx.reset_transform().unwrap();
        self.ctx.clear_rect(0.0, 0.0, self.width, self.height);
        self.ctx.set_fill_style_str("#11111b");
        self.ctx.fill_rect(0.0, 0.0, self.width, self.height);
        self.ctx.set_filter("none");
        self.fill_rule = CanvasWindingRule::Nonzero;
        self.current_transform = Affine::IDENTITY;
    }

    pub fn render_offscreen(&mut self) {}

    pub fn blit(&mut self) {}

    pub fn is_cpu(&self) -> bool {
        false
    }

    pub fn supports_encode_timing(&self) -> bool {
        false
    }

    pub fn sync(&self) {}

    pub fn resize(&mut self, w: u32, h: u32) {
        self.width = w as f64;
        self.height = h as f64;
        self.reset();
    }

    pub fn upload_image(&mut self, pixmap: Pixmap) -> ImageSource {
        let may_have_opacities = pixmap.may_have_opacities();
        let uploaded = UploadedImage::from_pixmap(pixmap);
        let id = ImageId::new(self.uploaded_images.len() as u32);
        self.uploaded_images.push(uploaded);
        ImageSource::opaque_id_with_opacity_hint(id, may_have_opacities)
    }

    pub fn set_paint(&mut self, paint: PaintType) {
        self.current_paint = match paint {
            PaintType::Solid(color) => PaintState::Solid(color.components),
            PaintType::Gradient(gradient) => PaintState::Gradient(gradient),
            PaintType::Image(image) => PaintState::Image(ImagePaint {
                image: image.image,
                quality: image.sampler.quality,
                alpha: image.sampler.alpha,
            }),
        };
    }

    pub fn set_transform(&mut self, transform: Affine) {
        self.current_transform = transform;
        let c = transform.as_coeffs();
        self.ctx
            .set_transform(c[0], c[1], c[2], c[3], c[4], c[5])
            .unwrap();
    }

    pub fn reset_transform(&mut self) {
        self.current_transform = Affine::IDENTITY;
        self.ctx.reset_transform().unwrap();
    }

    pub fn set_stroke(&mut self, stroke: Stroke) {
        self.ctx.set_line_width(stroke.width);
        self.ctx.set_miter_limit(stroke.miter_limit);
        self.ctx.set_line_cap(match stroke.start_cap {
            vello_common::kurbo::Cap::Butt => "butt",
            vello_common::kurbo::Cap::Round => "round",
            vello_common::kurbo::Cap::Square => "square",
        });
        self.ctx.set_line_join(match stroke.join {
            vello_common::kurbo::Join::Bevel => "bevel",
            vello_common::kurbo::Join::Miter => "miter",
            vello_common::kurbo::Join::Round => "round",
        });
    }

    pub fn set_paint_transform(&mut self, _transform: Affine) {}

    pub fn reset_paint_transform(&mut self) {}

    pub fn set_fill_rule(&mut self, fill: Fill) {
        self.fill_rule = match fill {
            Fill::NonZero => CanvasWindingRule::Nonzero,
            Fill::EvenOdd => CanvasWindingRule::Evenodd,
        };
    }

    pub fn fill_rect(&mut self, rect: &Rect) {
        if self.draw_current_image(rect) {
            return;
        }
        self.apply_fill_style();
        self.ctx
            .fill_rect(rect.x0, rect.y0, rect.width(), rect.height());
    }

    pub fn fill_path(&mut self, path: &BezPath) {
        self.apply_fill_style();
        self.ctx.begin_path();
        trace_path(&self.ctx, path);
        self.ctx.fill_with_canvas_winding_rule(self.fill_rule);
    }

    pub fn stroke_path(&mut self, path: &BezPath) {
        self.apply_stroke_style();
        self.ctx.begin_path();
        trace_path(&self.ctx, path);
        self.ctx.stroke();
    }

    pub fn push_clip_path(&mut self, path: &BezPath) {
        self.ctx.save();
        self.ctx.begin_path();
        trace_path(&self.ctx, path);
        self.ctx.clip_with_canvas_winding_rule(self.fill_rule);
        self.clip_depth += 1;
    }

    pub fn push_clip_layer(&mut self, path: &BezPath) {
        self.ctx.save();
        self.ctx.begin_path();
        trace_path(&self.ctx, path);
        self.ctx.clip_with_canvas_winding_rule(self.fill_rule);
        self.layer_stack.push(LayerKind::Clip);
    }

    pub fn push_filter_layer(&mut self, filter: Filter) {
        self.ctx.save();
        self.ctx
            .set_filter(&canvas_filter_string(&PreparedFilter::new(
                &filter,
                &self.current_transform,
            )));
        self.layer_stack.push(LayerKind::Filter);
    }

    pub fn pop_clip_path(&mut self) {
        if self.clip_depth > 0 {
            self.ctx.restore();
            self.clip_depth -= 1;
        }
    }

    pub fn pop_layer(&mut self) {
        if self.layer_stack.pop().is_some() {
            self.ctx.restore();
        }
    }

    pub fn fill_glyphs(
        &mut self,
        _font: &FontData,
        _font_size: f32,
        _hint: bool,
        _glyphs: &[Glyph],
    ) {
    }

    pub fn draw_text(
        &mut self,
        _font: &FontData,
        font_size: f32,
        _hint: bool,
        text: &str,
        x: f32,
        y: f32,
    ) {
        self.apply_fill_style();
        self.ctx.set_font(&format!("{font_size}px sans-serif"));
        self.ctx.set_text_baseline("alphabetic");
        let _ = self.ctx.fill_text(text, x as f64, y as f64);
    }

    pub fn draw_image(&mut self, image: ImageSource, rect: &Rect, bilinear: bool) {
        let Some(uploaded) = self.resolve_image(&image) else {
            return;
        };
        self.draw_uploaded_image(uploaded, rect, bilinear, 1.0);
    }

    fn draw_current_image(&self, rect: &Rect) -> bool {
        let PaintState::Image(image) = &self.current_paint else {
            return false;
        };
        let bilinear = !matches!(image.quality, ImageQuality::Low);
        let Some(uploaded) = self.resolve_image(&image.image) else {
            return false;
        };
        self.draw_uploaded_image(uploaded, rect, bilinear, image.alpha);
        true
    }

    fn resolve_image(&self, image: &ImageSource) -> Option<&UploadedImage> {
        match image {
            ImageSource::OpaqueId { id, .. } => self.uploaded_images.get(id.as_u32() as usize),
            ImageSource::Pixmap(_) => None,
        }
    }

    fn draw_uploaded_image(&self, image: &UploadedImage, rect: &Rect, bilinear: bool, alpha: f32) {
        self.ctx.set_image_smoothing_enabled(bilinear);
        let prev_alpha = self.ctx.global_alpha();
        if alpha != 1.0 {
            self.ctx.set_global_alpha(prev_alpha * alpha as f64);
        }
        let _ = self
            .ctx
            .draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                &image.canvas,
                0.0,
                0.0,
                image.width,
                image.height,
                rect.x0,
                rect.y0,
                rect.width(),
                rect.height(),
            );
        if alpha != 1.0 {
            self.ctx.set_global_alpha(prev_alpha);
        }
    }

    fn apply_fill_style(&self) {
        match &self.current_paint {
            PaintState::Solid(color) => self.ctx.set_fill_style_str(&css_rgba(color)),
            PaintState::Gradient(gradient) => {
                if let Some(canvas_gradient) = make_gradient(&self.ctx, gradient) {
                    self.ctx.set_fill_style_canvas_gradient(&canvas_gradient);
                }
            }
            PaintState::Image(_) => self.ctx.set_fill_style_str("rgba(0, 0, 0, 0)"),
        }
    }

    fn apply_stroke_style(&self) {
        match &self.current_paint {
            PaintState::Solid(color) => self.ctx.set_stroke_style_str(&css_rgba(color)),
            PaintState::Gradient(gradient) => {
                if let Some(canvas_gradient) = make_gradient(&self.ctx, gradient) {
                    self.ctx.set_stroke_style_canvas_gradient(&canvas_gradient);
                }
            }
            PaintState::Image(_) => self.ctx.set_stroke_style_str("rgba(0, 0, 0, 0)"),
        }
    }
}

impl UploadedImage {
    fn from_pixmap(pixmap: Pixmap) -> Self {
        let width = pixmap.width();
        let height = pixmap.height();
        let data = pixmap
            .take_unpremultiplied()
            .into_iter()
            .flat_map(|rgba| [rgba.r, rgba.g, rgba.b, rgba.a])
            .collect::<Vec<_>>();
        let image_data =
            ImageData::new_with_u8_clamped_array_and_sh(Clamped(&data), width as u32, height as u32)
                .unwrap();
        let document = window().unwrap().document().unwrap();
        let canvas: HtmlCanvasElement = document
            .create_element("canvas")
            .unwrap()
            .dyn_into()
            .unwrap();
        canvas.set_width(width as u32);
        canvas.set_height(height as u32);
        let ctx: CanvasRenderingContext2d = canvas
            .get_context("2d")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        ctx.put_image_data(&image_data, 0.0, 0.0).unwrap();
        Self {
            canvas,
            width: width as f64,
            height: height as f64,
        }
    }
}

fn trace_path(ctx: &CanvasRenderingContext2d, path: &BezPath) {
    for element in path.elements() {
        match *element {
            PathEl::MoveTo(p) => ctx.move_to(p.x, p.y),
            PathEl::LineTo(p) => ctx.line_to(p.x, p.y),
            PathEl::QuadTo(p1, p2) => ctx.quadratic_curve_to(p1.x, p1.y, p2.x, p2.y),
            PathEl::CurveTo(p1, p2, p3) => ctx.bezier_curve_to(p1.x, p1.y, p2.x, p2.y, p3.x, p3.y),
            PathEl::ClosePath => ctx.close_path(),
        }
    }
}

fn make_gradient(ctx: &CanvasRenderingContext2d, gradient: &Gradient) -> Option<CanvasGradient> {
    let canvas_gradient = match gradient.kind {
        GradientKind::Linear(LinearGradientPosition { start, end }) => {
            ctx.create_linear_gradient(start.x, start.y, end.x, end.y)
        }
        GradientKind::Radial(RadialGradientPosition {
            start_center,
            start_radius,
            end_center,
            end_radius,
        }) => ctx
            .create_radial_gradient(
                start_center.x,
                start_center.y,
                start_radius as f64,
                end_center.x,
                end_center.y,
                end_radius as f64,
            )
            .ok()?,
        GradientKind::Sweep(SweepGradientPosition {
            center,
            start_angle,
            ..
        }) => make_conic_gradient(ctx, start_angle as f64, center.x, center.y)?,
    };

    for stop in gradient.stops.0.iter() {
        let color = stop.color.to_alpha_color::<Srgb>();
        let _ = canvas_gradient.add_color_stop(stop.offset, &css_rgba(&color.components));
    }
    Some(canvas_gradient)
}

fn make_conic_gradient(
    ctx: &CanvasRenderingContext2d,
    start_angle: f64,
    x: f64,
    y: f64,
) -> Option<CanvasGradient> {
    let method = Reflect::get(ctx.as_ref(), &JsValue::from_str("createConicGradient")).ok()?;
    let function = method.dyn_into::<Function>().ok()?;
    function
        .call3(
            ctx.as_ref(),
            &JsValue::from_f64(start_angle),
            &JsValue::from_f64(x),
            &JsValue::from_f64(y),
        )
        .ok()?
        .dyn_into()
        .ok()
}

fn css_rgba(components: &[f32; 4]) -> String {
    let clamp = |value: f32| value.clamp(0.0, 1.0);
    let r = (clamp(components[0]) * 255.0).round() as u8;
    let g = (clamp(components[1]) * 255.0).round() as u8;
    let b = (clamp(components[2]) * 255.0).round() as u8;
    let a = clamp(components[3]);
    format!("rgba({r}, {g}, {b}, {a})")
}

fn canvas_filter_string(filter: &PreparedFilter) -> String {
    match filter {
        PreparedFilter::GaussianBlur(blur) => format!("blur({:.3}px)", blur.std_deviation),
        PreparedFilter::DropShadow(shadow) => format!(
            "drop-shadow({:.3}px {:.3}px {:.3}px {})",
            shadow.dx,
            shadow.dy,
            shadow.std_deviation,
            css_rgba(&shadow.color.components)
        ),
        _ => "none".to_string(),
    }
}
