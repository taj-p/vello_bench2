//! Backend abstraction over vello_hybrid (WebGL) and vello_cpu.
//!
//! `Backend` wraps the drawing context and renderer into a single type.
//! Drawing methods are forwarded to the inner context; backend-specific
//! operations (render, sync, image upload) live on `Backend` directly.

use vello_common::filter_effects::Filter;
use vello_common::glyph::Glyph;
use vello_common::kurbo::{Affine, BezPath, Rect, Stroke};
use vello_common::paint::{ImageSource, PaintType};
use vello_common::peniko::{Fill, FontData};
use web_sys::HtmlCanvasElement;

use crate::scenes::{ParamId, SceneId};

// ── CPU backend ──────────────────────────────────────────────────────────────

#[cfg(feature = "cpu")]
mod inner {
    use alloc::sync::Arc;
    use vello_common::paint::ImageSource;
    pub use vello_cpu::Pixmap;
    use wasm_bindgen::JsCast;
    use web_sys::{HtmlCanvasElement, WebGl2RenderingContext as GL, WebGlProgram, WebGlTexture};

    extern crate alloc;

    const VS: &str = "\
        attribute vec2 p;\
        varying vec2 uv;\
        void main(){uv=p*0.5+0.5;uv.y=1.0-uv.y;gl_Position=vec4(p,0,1);}";
    const FS: &str = "\
        precision mediump float;\
        varying vec2 uv;\
        uniform sampler2D t;\
        void main(){gl_FragColor=texture2D(t,uv);}";

    pub type DrawContext = vello_cpu::RenderContext;

    pub struct BackendInner {
        width: u16,
        height: u16,
        gl: GL,
        #[allow(dead_code)]
        program: WebGlProgram,
        texture: WebGlTexture,
        target: Option<Pixmap>,
    }

    impl std::fmt::Debug for BackendInner {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Backend(cpu)").finish()
        }
    }

    impl BackendInner {
        pub fn new(canvas: &HtmlCanvasElement) -> Self {
            let gl: GL = canvas
                .get_context("webgl2")
                .unwrap()
                .unwrap()
                .dyn_into()
                .unwrap();

            let vs = gl.create_shader(GL::VERTEX_SHADER).unwrap();
            gl.shader_source(&vs, VS);
            gl.compile_shader(&vs);
            let fs = gl.create_shader(GL::FRAGMENT_SHADER).unwrap();
            gl.shader_source(&fs, FS);
            gl.compile_shader(&fs);
            let program = gl.create_program().unwrap();
            gl.attach_shader(&program, &vs);
            gl.attach_shader(&program, &fs);
            gl.link_program(&program);
            gl.delete_shader(Some(&vs));
            gl.delete_shader(Some(&fs));

            let verts: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
            let buf = gl.create_buffer().unwrap();
            gl.bind_buffer(GL::ARRAY_BUFFER, Some(&buf));
            let view = js_sys::Float32Array::new_with_length(8);
            view.copy_from(&verts);
            gl.buffer_data_with_array_buffer_view(GL::ARRAY_BUFFER, &view, GL::STATIC_DRAW);
            let loc = gl.get_attrib_location(&program, "p") as u32;
            gl.enable_vertex_attrib_array(loc);
            gl.vertex_attrib_pointer_with_i32(loc, 2, GL::FLOAT, false, 0, 0);

            let texture = gl.create_texture().unwrap();
            gl.bind_texture(GL::TEXTURE_2D, Some(&texture));
            gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MIN_FILTER, GL::NEAREST as i32);
            gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MAG_FILTER, GL::NEAREST as i32);

            gl.use_program(Some(&program));
            gl.disable(GL::BLEND);

            let w = canvas.width() as u16;
            let h = canvas.height() as u16;

            Self {
                width: w,
                height: h,
                gl,
                program,
                texture,
                target: None,
            }
        }

        /// CPU rendering: flush + render to pixmap.
        pub fn render_offscreen(&mut self, ctx: &mut DrawContext) {
            ctx.flush();
            self.target = Some(Pixmap::new(self.width, self.height));
            ctx.render_to_pixmap(self.target.as_mut().unwrap());
        }

        /// Blit the rendered pixmap to the canvas via WebGL2.
        pub fn blit(&mut self) {
            let target = self.target.take().expect("render_offscreen not called");
            let bytes: &[u8] = bytemuck::cast_slice(target.data());
            let gl = &self.gl;

            gl.bind_texture(GL::TEXTURE_2D, Some(&self.texture));
            gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
                GL::TEXTURE_2D,
                0,
                GL::RGBA as i32,
                self.width as i32,
                self.height as i32,
                0,
                GL::RGBA,
                GL::UNSIGNED_BYTE,
                Some(bytes),
            )
            .unwrap();

            gl.viewport(0, 0, self.width as i32, self.height as i32);
            gl.draw_arrays(GL::TRIANGLE_STRIP, 0, 4);
        }

        pub fn resize(&mut self, w: u32, h: u32) {
            self.width = w as u16;
            self.height = h as u16;
        }

        pub fn upload_image(&mut self, _ctx: &mut DrawContext, pixmap: Pixmap) -> ImageSource {
            ImageSource::Pixmap(Arc::new(pixmap))
        }

        pub fn sync(&self) {}
    }
}

// ── Hybrid (WebGL) backend ───────────────────────────────────────────────────

#[cfg(feature = "pathfinder")]
mod inner {
    use pathfinder_canvas::{Canvas, CanvasFontContext, CanvasRenderingContext2D, ColorU, RectF};
    use pathfinder_geometry::transform2d::Transform2F;
    use pathfinder_geometry::vector::{Vector2F, vec2i};
    use pathfinder_renderer::concurrent::executor::SequentialExecutor;
    use pathfinder_renderer::gpu::options::{DestFramebuffer, RendererMode, RendererOptions};
    use pathfinder_renderer::gpu::renderer::Renderer as PathfinderRenderer;
    use pathfinder_renderer::options::BuildOptions;
    use pathfinder_resources::embedded::EmbeddedResourceLoader;
    use pathfinder_webgl::WebGlDevice;
    use vello_common::filter_effects::Filter;
    use vello_common::glyph::Glyph;
    use vello_common::kurbo::{Affine, BezPath, Rect, Stroke};
    use vello_common::paint::{ImageSource, PaintType};
    use vello_common::peniko::{Fill, FontData};
    use wasm_bindgen::JsCast;
    use web_sys::{HtmlCanvasElement, WebGl2RenderingContext};

    pub struct Pixmap;

    impl Pixmap {
        pub fn from_parts_with_opacity<T>(
            _pixels: Vec<T>,
            _width: u16,
            _height: u16,
            _may_have_opacities: bool,
        ) -> Self {
            Self
        }
    }

    pub struct DrawContext {
        width: u16,
        height: u16,
        canvas: Option<CanvasRenderingContext2D>,
        fill_color: ColorU,
    }

    impl DrawContext {
        pub fn new(width: u16, height: u16) -> Self {
            let mut ctx = Self {
                width,
                height,
                canvas: None,
                fill_color: ColorU::black(),
            };
            ctx.reset();
            ctx
        }

        pub fn reset(&mut self) {
            let font_context = CanvasFontContext::from_system_source();
            self.canvas = Some(
                Canvas::new(Vector2F::new(self.width as f32, self.height as f32))
                    .get_context_2d(font_context),
            );
        }

        pub fn set_paint(&mut self, paint: PaintType) {
            if let PaintType::Solid(color) = paint {
                let [r, g, b, a] = color.to_rgba8().to_u8_array();
                self.fill_color = ColorU::new(r, g, b, a);
            }
        }

        pub fn set_transform(&mut self, transform: Affine) {
            if let Some(canvas) = self.canvas.as_mut() {
                let c = transform.as_coeffs();
                canvas.set_transform(&Transform2F::row_major(
                    c[0] as f32,
                    c[2] as f32,
                    c[4] as f32,
                    c[1] as f32,
                    c[3] as f32,
                    c[5] as f32,
                ));
            }
        }

        pub fn reset_transform(&mut self) {
            if let Some(canvas) = self.canvas.as_mut() {
                canvas.reset_transform();
            }
        }

        pub fn set_stroke(&mut self, _stroke: Stroke) {}
        pub fn set_paint_transform(&mut self, _transform: Affine) {}
        pub fn reset_paint_transform(&mut self) {}
        pub fn set_fill_rule(&mut self, _fill: Fill) {}

        pub fn fill_rect(&mut self, rect: &Rect) {
            if let Some(canvas) = self.canvas.as_mut() {
                canvas.set_fill_style(self.fill_color);
                canvas.fill_rect(RectF::new(
                    Vector2F::new(rect.x0 as f32, rect.y0 as f32),
                    Vector2F::new(rect.width() as f32, rect.height() as f32),
                ));
            }
        }

        pub fn fill_path(&mut self, _path: &BezPath) {}
        pub fn stroke_path(&mut self, _path: &BezPath) {}
        pub fn push_clip_path(&mut self, _path: &BezPath) {}
        pub fn push_clip_layer(&mut self, _path: &BezPath) {}
        pub fn push_filter_layer(&mut self, _filter: Filter) {}
        pub fn pop_clip_path(&mut self) {}
        pub fn pop_layer(&mut self) {}
        pub fn fill_glyphs(&mut self, _font: &FontData, _font_size: f32, _hint: bool, _glyphs: &[Glyph]) {}
    }

    pub struct BackendInner {
        renderer: PathfinderRenderer<WebGlDevice>,
    }

    impl std::fmt::Debug for BackendInner {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Backend(pathfinder)").finish()
        }
    }

    impl BackendInner {
        pub fn new(canvas: &HtmlCanvasElement) -> Self {
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
                background_color: Some(pathfinder_canvas::ColorF::new(
                    17.0 / 255.0,
                    17.0 / 255.0,
                    27.0 / 255.0,
                    1.0,
                )),
                ..RendererOptions::default()
            };
            let loader = EmbeddedResourceLoader::new();
            let renderer = PathfinderRenderer::new(device, &loader, mode, options);
            Self { renderer }
        }

        pub fn render_offscreen(&mut self, ctx: &mut DrawContext) {
            if let Some(canvas) = ctx.canvas.take() {
                let mut scene = canvas.into_canvas().into_scene();
                scene.build_and_render(
                    &mut self.renderer,
                    BuildOptions::default(),
                    SequentialExecutor,
                );
            }
        }

        pub fn blit(&mut self) {}

        pub fn resize(&mut self, w: u32, h: u32) {
            self.renderer.options_mut().dest = DestFramebuffer::full_window(vec2i(w as i32, h as i32));
            self.renderer.dest_framebuffer_size_changed();
        }

        pub fn upload_image(&mut self, _ctx: &mut DrawContext, _pixmap: Pixmap) -> ImageSource {
            panic!("pathfinder image upload not implemented")
        }

        pub fn sync(&self) {}
    }
}

#[cfg(all(not(feature = "cpu"), not(feature = "pathfinder")))]
mod inner {
    use vello_common::paint::ImageSource;
    pub use vello_hybrid::Pixmap;
    use web_sys::HtmlCanvasElement;

    pub type DrawContext = vello_hybrid::Scene;

    pub struct BackendInner {
        renderer: vello_hybrid::WebGlRenderer,
    }

    impl std::fmt::Debug for BackendInner {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Backend(hybrid)").finish()
        }
    }

    impl BackendInner {
        pub fn new(canvas: &HtmlCanvasElement) -> Self {
            Self {
                renderer: vello_hybrid::WebGlRenderer::new(canvas),
            }
        }

        pub fn render_offscreen(&mut self, ctx: &mut DrawContext) {
            let rs = vello_hybrid::RenderSize {
                width: ctx.width() as u32,
                height: ctx.height() as u32,
            };
            self.renderer.render(ctx, &rs).unwrap();
        }

        pub fn blit(&mut self) {
            // No-op: hybrid renders directly to the canvas.
        }

        pub fn resize(&mut self, _w: u32, _h: u32) {}

        pub fn upload_image(&mut self, _ctx: &mut DrawContext, pixmap: Pixmap) -> ImageSource {
            let id = self.renderer.upload_image(&pixmap);
            ImageSource::opaque_id_with_opacity_hint(id, pixmap.may_have_opacities())
        }

        pub fn sync(&self) {
            crate::gpu_sync(&self.renderer);
        }
    }
}

pub use inner::Pixmap;
use inner::{BackendInner, DrawContext};

// ── Scene-facing abstraction ────────────────────────────────────────────────

pub trait Renderer {
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
    pub fn supports_scene(self, _scene_id: SceneId) -> bool {
        #[cfg(feature = "pathfinder")]
        {
            matches!(_scene_id, SceneId::Rect)
        }
        #[cfg(not(feature = "pathfinder"))]
        {
            true
        }
    }

    pub fn supports_param(self, _scene_id: SceneId, _param: ParamId) -> bool {
        #[cfg(feature = "pathfinder")]
        {
            matches!(
                (_scene_id, _param),
                (SceneId::Rect, ParamId::NumRects)
                    | (SceneId::Rect, ParamId::RectSize)
                    | (SceneId::Rect, ParamId::Rotated)
            )
        }
        #[cfg(all(not(feature = "pathfinder"), feature = "cpu"))]
        {
            if _scene_id == SceneId::Rect && _param == ParamId::UseDrawImage {
                return false;
            }
            true
        }
        #[cfg(all(not(feature = "pathfinder"), not(feature = "cpu")))]
        {
            true
        }
    }
}

pub fn current_backend_capabilities() -> BackendCapabilities {
    BackendCapabilities
}

// ── Unified Backend ──────────────────────────────────────────────────────────

pub struct Backend {
    ctx: DrawContext,
    inner: BackendInner,
}

impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(f)
    }
}

impl Backend {
    pub fn new(canvas: &HtmlCanvasElement, w: u32, h: u32) -> Self {
        let inner = BackendInner::new(canvas);
        let ctx = DrawContext::new(w as u16, h as u16);
        Self { ctx, inner }
    }

    /// Reset the draw context for a new frame.
    pub fn reset(&mut self) {
        self.ctx.reset();
    }

    /// Create a fresh draw context (e.g. for benchmark isolation).
    pub fn reset_with_size(&mut self, w: u32, h: u32) {
        self.ctx = DrawContext::new(w as u16, h as u16);
    }

    /// Render offscreen (CPU: render_to_pixmap, Hybrid: full GPU render).
    pub fn render_offscreen(&mut self) {
        self.inner.render_offscreen(&mut self.ctx);
    }

    /// Blit to canvas (CPU: texImage2D + draw, Hybrid: no-op).
    pub fn blit(&mut self) {
        self.inner.blit();
    }

    pub fn is_cpu(&self) -> bool {
        cfg!(feature = "cpu")
    }

    /// Synchronize (wait for GPU on hybrid, no-op on CPU).
    pub fn sync(&self) {
        self.inner.sync();
    }

    /// Notify the backend of a canvas resize.
    pub fn resize(&mut self, w: u32, h: u32) {
        self.inner.resize(w, h);
        self.ctx = DrawContext::new(w as u16, h as u16);
    }

    fn upload_image_impl(&mut self, pixmap: Pixmap) -> ImageSource {
        self.inner.upload_image(&mut self.ctx, pixmap)
    }

    fn draw_image_impl(&mut self, _image: ImageSource, _rect: &Rect, _bilinear: bool) {
        // TODO: Re-add this once bilinear image painting has been merged to main.
        // #[cfg(not(feature = "cpu"))]
        // {
        //     self.ctx.draw_image(image, rect, bilinear);
        // }
        // #[cfg(feature = "cpu")]
        // {
        //     use vello_common::paint::Image;
        //     use vello_common::peniko::{Extend, ImageQuality, ImageSampler};
        //     let old_paint_transform = *self.ctx.paint_transform();
        //     let old_paint = self.ctx.paint().clone();
        //     self.ctx.set_paint_transform(Affine::IDENTITY);
        //     self.ctx.set_paint(Image {
        //         image,
        //         sampler: ImageSampler {
        //             x_extend: Extend::Pad,
        //             y_extend: Extend::Pad,
        //             quality: if bilinear {
        //                 ImageQuality::Medium
        //             } else {
        //                 ImageQuality::Low
        //             },
        //             alpha: 1.0,
        //         },
        //     });
        //     self.ctx.fill_rect(rect);
        //     self.ctx.set_paint_transform(old_paint_transform);
        //     self.ctx.set_paint(old_paint);
        // }
    }
}

impl Renderer for Backend {
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

    fn push_filter_layer(&mut self, filter: Filter) {
        self.ctx.push_filter_layer(filter);
    }

    fn pop_clip_path(&mut self) {
        self.ctx.pop_clip_path();
    }

    fn pop_layer(&mut self) {
        self.ctx.pop_layer();
    }

    fn fill_glyphs(&mut self, font: &FontData, font_size: f32, hint: bool, glyphs: &[Glyph]) {
        #[cfg(feature = "pathfinder")]
        {
            self.ctx.fill_glyphs(font, font_size, hint, glyphs);
        }
        #[cfg(not(feature = "pathfinder"))]
        {
        self.ctx
            .glyph_run(font)
            .font_size(font_size)
            .hint(hint)
            .fill_glyphs(glyphs.iter().copied());
        }
    }

    fn draw_image(&mut self, image: ImageSource, rect: &Rect, bilinear: bool) {
        self.draw_image_impl(image, rect, bilinear);
    }

    fn upload_image(&mut self, pixmap: Pixmap) -> ImageSource {
        self.upload_image_impl(pixmap)
    }
}
