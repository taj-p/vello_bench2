//! Backend abstraction over vello_hybrid (WebGL) and vello_cpu.
//!
//! `Backend` wraps the drawing context and renderer into a single type.
//! Drawing methods are forwarded to the inner context; backend-specific
//! operations (render, sync, image upload) live on `Backend` directly.

use vello_common::filter_effects::Filter;
use vello_common::kurbo::{Affine, BezPath, Rect, Stroke};
use vello_common::paint::{ImageSource, PaintType};
use vello_common::peniko::{Fill, FontData};
use web_sys::HtmlCanvasElement;

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

#[cfg(not(feature = "cpu"))]
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

    /// Upload an image and return its ID.
    pub fn upload_image(&mut self, pixmap: Pixmap) -> ImageSource {
        self.inner.upload_image(&mut self.ctx, pixmap)
    }

    // ── Drawing methods (forwarded to inner DrawContext) ─────────────────

    pub fn set_paint(&mut self, paint: impl Into<PaintType>) {
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

    pub fn glyph_run(
        &mut self,
        font: &FontData,
    ) -> vello_common::glyph::GlyphRunBuilder<'_, DrawContext> {
        self.ctx.glyph_run(font)
    }

    /// Draw an image into a rect.
    ///
    /// On the hybrid backend this uses the GPU fast path (`Scene::draw_image`).
    /// On the CPU backend this falls back to image paint with Pad extend.
    pub fn draw_image(&mut self, image: ImageSource, rect: &Rect, bilinear: bool) {
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
