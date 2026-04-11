use pathfinder_canvas::{
    Canvas, CanvasFontContext, CanvasRenderingContext2D, ColorU, FillRule, ImageData, LineCap,
    LineJoin, Path2D, RectF,
};
use pathfinder_content::gradient::Gradient as PathfinderGradient;
use pathfinder_content::pattern::{Image as PathfinderImage, Pattern};
use pathfinder_geometry::line_segment::LineSegment2F;
use pathfinder_geometry::transform2d::Transform2F;
use pathfinder_geometry::vector::{Vector2F, vec2f, vec2i};
use pathfinder_renderer::concurrent::executor::SequentialExecutor;
use pathfinder_renderer::gpu::options::{DestFramebuffer, RendererMode, RendererOptions};
use pathfinder_renderer::gpu::renderer::Renderer as PathfinderRenderer;
use pathfinder_renderer::options::BuildOptions;
use pathfinder_renderer::scene::Scene;
use pathfinder_resources::embedded::EmbeddedResourceLoader;
use pathfinder_simd::default::F32x2;
use pathfinder_webgl::WebGlDevice;
use vello_common::filter_effects::Filter;
use vello_common::kurbo::{Affine, BezPath, PathEl, Rect, Stroke};
use vello_common::paint::{ImageId, ImageSource, PaintType};
use vello_common::peniko::{
    Fill, FontData, Gradient, GradientKind, LinearGradientPosition, RadialGradientPosition,
};
use vello_common::pixmap::Pixmap;
use wasm_bindgen::JsCast;
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext};

use crate::capability::{CapabilityProfile, UnsupportedParamValue};
use crate::scenes::{ParamId, SceneId};

const UNSUPPORTED_VALUES: &[UnsupportedParamValue] = &[UnsupportedParamValue::new(
    SceneId::Rect,
    ParamId::GradientShape,
    2,
)];

pub(crate) const CAPABILITIES: CapabilityProfile = CapabilityProfile::none()
    .allow_scenes(&[
        SceneId::Rect,
        SceneId::Strokes,
        SceneId::Polyline,
        SceneId::Svg,
        SceneId::Clip,
    ])
    .allow_params(
        SceneId::Rect,
        &[
            ParamId::NumRects,
            ParamId::PaintMode,
            ParamId::RectSize,
            ParamId::Rotated,
            ParamId::ImageFilter,
            ParamId::ImageOpaque,
            ParamId::UseDrawImage,
            ParamId::GradientShape,
            ParamId::DynamicGradient,
        ],
    )
    .allow_params(
        SceneId::Strokes,
        &[
            ParamId::NumStrokes,
            ParamId::CurveType,
            ParamId::Segments,
            ParamId::StrokeWidth,
            ParamId::Cap,
        ],
    )
    .allow_params(SceneId::Polyline, &[ParamId::NumVertices])
    .allow_params(SceneId::Svg, &[ParamId::SvgAsset])
    .allow_params(
        SceneId::Clip,
        &[
            ParamId::NumRects,
            ParamId::RectSize,
            ParamId::ClipMode,
            ParamId::ClipMethod,
        ],
    )
    .with_unsupported_values(UNSUPPORTED_VALUES);

pub struct BackendImpl {
    ctx: DrawContext,
    renderer: PathfinderRenderer<WebGlDevice>,
    uploaded_images: Vec<UploadedImage>,
}

impl std::fmt::Debug for BackendImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backend(pathfinder)").finish()
    }
}

impl BackendImpl {
    pub fn new(canvas: &HtmlCanvasElement, w: u32, h: u32) -> Self {
        let context: WebGl2RenderingContext = canvas
            .get_context("webgl2")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        let device = WebGlDevice::new(context);
        let framebuffer_size = vec2i(canvas.width() as i32, canvas.height() as i32);
        let mode = RendererMode::default_for_device(&device);
        let options = RendererOptions {
            dest: DestFramebuffer::full_window(framebuffer_size),
            background_color: None,
            ..RendererOptions::default()
        };
        let loader = EmbeddedResourceLoader::new();
        Self {
            ctx: DrawContext::new(w as u16, h as u16),
            renderer: PathfinderRenderer::new(device, &loader, mode, options),
            uploaded_images: Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        self.ctx.reset();
    }

    pub fn render_offscreen(&mut self) {
        let mut scene = self.ctx.take_scene();
        scene.build_and_render(
            &mut self.renderer,
            BuildOptions::default(),
            SequentialExecutor,
        );
    }

    pub fn blit(&mut self) {}

    pub fn is_cpu(&self) -> bool {
        false
    }

    pub fn supports_encode_timing(&self) -> bool {
        false
    }

    pub fn sync(&self) {}

    pub fn resize(&mut self, w: u32, h: u32) {
        self.ctx.resize(w as u16, h as u16);
        self.renderer.options_mut().dest = DestFramebuffer::full_window(vec2i(w as i32, h as i32));
        self.renderer.options_mut().background_color = None;
        self.renderer.dest_framebuffer_size_changed();
    }

    pub fn upload_image(&mut self, pixmap: Pixmap) -> ImageSource {
        let may_have_opacities = pixmap.may_have_opacities();
        let id = ImageId::new(self.uploaded_images.len() as u32);
        self.uploaded_images
            .push(UploadedImage::from_pixmap(pixmap));
        ImageSource::opaque_id_with_opacity_hint(id, may_have_opacities)
    }

    pub fn set_paint(&mut self, paint: PaintType) {
        self.ctx.set_paint(paint, &self.uploaded_images);
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

    pub fn set_paint_transform(&mut self, _transform: Affine) {}

    pub fn reset_paint_transform(&mut self) {}

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

    pub fn set_filter_effect(&mut self, filter: Filter) {
        let _ = filter;
        // Pathfinder filter support is intentionally disabled. The shadow-based
        // approximation was visually incorrect and made FilterLayers look buggy.
    }

    pub fn pop_clip_path(&mut self) {
        self.ctx.pop_clip_path();
    }

    pub fn pop_layer(&mut self) {
        self.ctx.pop_layer();
    }

    pub fn draw_text(
        &mut self,
        _font: &FontData,
        _font_size: f32,
        _hint: bool,
        _text: &str,
        _x: f32,
        _y: f32,
    ) {
        // Pathfinder text stays disabled in the wasm build. Enabling `pathfinder_canvas`
        // `pf-text` support pulls in freetype/harfbuzz native dependencies, which fail
        // to build for our `wasm32-unknown-unknown` serve/build pipeline.
    }

    pub fn draw_image(&mut self, image: ImageSource, rect: &Rect, bilinear: bool) {
        self.ctx
            .draw_image(image, rect, bilinear, &self.uploaded_images);
    }
}

struct DrawContext {
    width: u16,
    height: u16,
    font_context: CanvasFontContext,
    canvas: CanvasRenderingContext2D,
    current_paint: PaintState,
    fill_rule: FillRule,
    clip_depth: usize,
}

#[derive(Clone)]
enum PaintState {
    Solid(ColorU),
    Gradient(PathfinderGradient),
    Image(ImagePaint),
}

#[derive(Clone)]
struct ImagePaint {
    image: PathfinderImage,
    bilinear: bool,
    alpha: f32,
}

struct UploadedImage {
    image: PathfinderImage,
}

impl DrawContext {
    fn new(width: u16, height: u16) -> Self {
        let font_context = CanvasFontContext::from_system_source();
        let mut ctx = Self {
            width,
            height,
            canvas: make_canvas_context(width, height, font_context.clone()),
            font_context,
            current_paint: PaintState::Solid(ColorU::black()),
            fill_rule: FillRule::Winding,
            clip_depth: 0,
        };
        ctx.reset();
        ctx
    }

    fn reset(&mut self) {
        self.canvas = make_canvas_context(self.width, self.height, self.font_context.clone());
        self.current_paint = PaintState::Solid(ColorU::black());
        self.fill_rule = FillRule::Winding;
        self.clip_depth = 0;
    }

    fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        self.reset();
    }

    fn take_scene(&mut self) -> Scene {
        self.canvas.canvas_mut().take_scene()
    }

    fn set_paint(&mut self, paint: PaintType, uploaded_images: &[UploadedImage]) {
        self.current_paint = match paint {
            PaintType::Solid(color) => {
                let [r, g, b, a] = color.to_rgba8().to_u8_array();
                PaintState::Solid(ColorU::new(r, g, b, a))
            }
            PaintType::Gradient(gradient) => match to_pathfinder_gradient(&gradient) {
                Some(gradient) => PaintState::Gradient(gradient),
                None => PaintState::Solid(ColorU::transparent_black()),
            },
            PaintType::Image(image) => {
                let Some(uploaded) = resolve_uploaded_image(uploaded_images, &image.image) else {
                    return;
                };
                PaintState::Image(ImagePaint {
                    image: uploaded.image.clone(),
                    bilinear: !matches!(
                        image.sampler.quality,
                        vello_common::peniko::ImageQuality::Low
                    ),
                    alpha: image.sampler.alpha,
                })
            }
        };
    }

    fn set_transform(&mut self, transform: Affine) {
        let c = transform.as_coeffs();
        self.canvas.set_transform(&Transform2F::row_major(
            c[0] as f32,
            c[2] as f32,
            c[4] as f32,
            c[1] as f32,
            c[3] as f32,
            c[5] as f32,
        ));
    }

    fn reset_transform(&mut self) {
        self.canvas.reset_transform();
    }

    fn set_fill_rule(&mut self, fill: Fill) {
        self.fill_rule = match fill {
            Fill::EvenOdd => FillRule::EvenOdd,
            Fill::NonZero => FillRule::Winding,
        };
    }

    fn set_stroke(&mut self, stroke: Stroke) {
        self.canvas.set_line_width(stroke.width as f32);
        self.canvas.set_miter_limit(stroke.miter_limit as f32);
        self.canvas.set_line_cap(match stroke.start_cap {
            vello_common::kurbo::Cap::Butt => LineCap::Butt,
            vello_common::kurbo::Cap::Square => LineCap::Square,
            vello_common::kurbo::Cap::Round => LineCap::Round,
        });
        self.canvas.set_line_join(match stroke.join {
            vello_common::kurbo::Join::Bevel => LineJoin::Bevel,
            vello_common::kurbo::Join::Miter => LineJoin::Miter,
            vello_common::kurbo::Join::Round => LineJoin::Round,
        });
    }

    fn fill_rect(&mut self, rect: &Rect) {
        let rectf = RectF::new(
            Vector2F::new(rect.x0 as f32, rect.y0 as f32),
            Vector2F::new(rect.width() as f32, rect.height() as f32),
        );
        match &self.current_paint {
            PaintState::Solid(fill_color) => {
                self.canvas.set_fill_style(*fill_color);
                self.canvas.fill_rect(rectf);
            }
            PaintState::Gradient(gradient) => {
                self.canvas.set_fill_style(gradient.clone());
                self.canvas.fill_rect(rectf);
            }
            PaintState::Image(image) => {
                draw_pathfinder_image(&mut self.canvas, image, rectf);
            }
        }
    }

    fn fill_path(&mut self, path: &BezPath) {
        match &self.current_paint {
            PaintState::Solid(fill_color) => {
                self.canvas.set_fill_style(*fill_color);
                self.canvas.fill_path(path_to_path2d(path), self.fill_rule);
            }
            PaintState::Gradient(gradient) => {
                self.canvas.set_fill_style(gradient.clone());
                self.canvas.fill_path(path_to_path2d(path), self.fill_rule);
            }
            PaintState::Image(_) => {}
        }
    }

    fn stroke_path(&mut self, path: &BezPath) {
        match &self.current_paint {
            PaintState::Solid(fill_color) => {
                self.canvas.set_stroke_style(*fill_color);
                self.canvas.stroke_path(path_to_path2d(path));
            }
            PaintState::Gradient(gradient) => {
                self.canvas.set_stroke_style(gradient.clone());
                self.canvas.stroke_path(path_to_path2d(path));
            }
            PaintState::Image(_) => {}
        }
    }

    fn push_clip_path(&mut self, path: &BezPath) {
        self.canvas.save();
        self.canvas.clip_path(path_to_path2d(path), self.fill_rule);
        self.clip_depth += 1;
    }

    fn push_clip_layer(&mut self, path: &BezPath) {
        self.push_clip_path(path);
    }

    fn pop_clip_path(&mut self) {
        if self.clip_depth > 0 {
            self.canvas.restore();
            self.clip_depth -= 1;
        }
    }

    fn pop_layer(&mut self) {
        self.pop_clip_path();
    }

    fn draw_image(
        &mut self,
        image: ImageSource,
        rect: &Rect,
        bilinear: bool,
        uploaded_images: &[UploadedImage],
    ) {
        let Some(uploaded) = resolve_uploaded_image(uploaded_images, &image) else {
            return;
        };
        draw_pathfinder_image(
            &mut self.canvas,
            &ImagePaint {
                image: uploaded.image.clone(),
                bilinear,
                alpha: 1.0,
            },
            RectF::new(
                Vector2F::new(rect.x0 as f32, rect.y0 as f32),
                Vector2F::new(rect.width() as f32, rect.height() as f32),
            ),
        );
    }
}

fn make_canvas_context(
    width: u16,
    height: u16,
    font_context: CanvasFontContext,
) -> CanvasRenderingContext2D {
    Canvas::new(Vector2F::new(width as f32, height as f32)).get_context_2d(font_context)
}

fn to_pathfinder_gradient(gradient: &Gradient) -> Option<PathfinderGradient> {
    let mut out = match gradient.kind {
        GradientKind::Linear(LinearGradientPosition { start, end }) => {
            PathfinderGradient::linear(LineSegment2F::new(
                vec2f(start.x as f32, start.y as f32),
                vec2f(end.x as f32, end.y as f32),
            ))
        }
        GradientKind::Radial(RadialGradientPosition {
            start_center,
            start_radius,
            end_center,
            end_radius,
        }) => PathfinderGradient::radial(
            LineSegment2F::new(
                vec2f(start_center.x as f32, start_center.y as f32),
                vec2f(end_center.x as f32, end_center.y as f32),
            ),
            F32x2::new(start_radius, end_radius),
        ),
        GradientKind::Sweep(_) => return None,
    };
    for stop in gradient.stops.0.iter() {
        let color = stop
            .color
            .to_alpha_color::<vello_common::peniko::color::Srgb>();
        let [r, g, b, a] = color.to_rgba8().to_u8_array();
        out.add_color_stop(ColorU::new(r, g, b, a), stop.offset as f32);
    }
    Some(out)
}

impl UploadedImage {
    fn from_pixmap(pixmap: Pixmap) -> Self {
        let width = pixmap.width();
        let height = pixmap.height();
        let data = pixmap
            .take_unpremultiplied()
            .into_iter()
            .map(|rgba| ColorU::new(rgba.r, rgba.g, rgba.b, rgba.a))
            .collect();
        let image = ImageData {
            data,
            size: vec2i(width as i32, height as i32),
        }
        .into_image();
        Self { image }
    }
}

fn resolve_uploaded_image<'a>(
    uploaded_images: &'a [UploadedImage],
    image: &ImageSource,
) -> Option<&'a UploadedImage> {
    match image {
        ImageSource::OpaqueId { id, .. } => uploaded_images.get(id.as_u32() as usize),
        ImageSource::Pixmap(_) => None,
    }
}

fn draw_pathfinder_image(canvas: &mut CanvasRenderingContext2D, image: &ImagePaint, rect: RectF) {
    let mut pattern = Pattern::from_image(image.image.clone());
    pattern.set_smoothing_enabled(image.bilinear);
    let old_alpha = canvas.global_alpha();
    if image.alpha != 1.0 {
        canvas.set_global_alpha(old_alpha * image.alpha);
    }
    canvas.draw_image(pattern, rect);
    if image.alpha != 1.0 {
        canvas.set_global_alpha(old_alpha);
    }
}

fn path_to_path2d(path: &BezPath) -> Path2D {
    let mut out = Path2D::new();
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => out.move_to(vec2f(p.x as f32, p.y as f32)),
            PathEl::LineTo(p) => out.line_to(vec2f(p.x as f32, p.y as f32)),
            PathEl::QuadTo(p1, p2) => out.quadratic_curve_to(
                vec2f(p1.x as f32, p1.y as f32),
                vec2f(p2.x as f32, p2.y as f32),
            ),
            PathEl::CurveTo(p1, p2, p3) => out.bezier_curve_to(
                vec2f(p1.x as f32, p1.y as f32),
                vec2f(p2.x as f32, p2.y as f32),
                vec2f(p3.x as f32, p3.y as f32),
            ),
            PathEl::ClosePath => out.close_path(),
        }
    }
    out
}
