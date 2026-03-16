//! Backend abstraction over vello_hybrid (WebGL) and vello_cpu.
//!
//! Both backends share the same scene-building API from vello_common,
//! so the abstraction is thin — mainly around initialization, rendering,
//! and image upload.

#[cfg(feature = "cpu")]
mod inner {
    use alloc::sync::Arc;
    use vello_common::paint::ImageId;
    pub use vello_cpu::Pixmap;
    pub use vello_cpu::RenderContext as DrawContext;
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

    pub struct Backend {
        width: u16,
        height: u16,
        gl: GL,
        /// Kept alive to prevent the browser from garbage-collecting the GL program.
        #[allow(dead_code)]
        program: WebGlProgram,
        texture: WebGlTexture,
    }

    impl std::fmt::Debug for Backend {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Backend(cpu)").finish()
        }
    }

    fn pixmap_as_bytes(pixmap: &Pixmap) -> &[u8] {
        bytemuck::cast_slice(pixmap.data())
    }

    impl Backend {
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
            }
        }

        pub fn render(&mut self, ctx: &mut DrawContext) {
            ctx.flush();
            let mut target = Pixmap::new(self.width, self.height);
            ctx.render_to_pixmap(&mut target);

            let bytes = pixmap_as_bytes(&target);
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

        pub fn upload_image(&mut self, ctx: &mut DrawContext, pixmap: Pixmap) -> ImageId {
            ctx.register_image(Arc::new(pixmap))
        }

        pub fn sync(&self) {}
    }
}

#[cfg(not(feature = "cpu"))]
mod inner {
    use vello_common::paint::ImageId;
    pub use vello_hybrid::Pixmap;
    pub use vello_hybrid::Scene as DrawContext;
    use web_sys::HtmlCanvasElement;

    pub struct Backend {
        renderer: vello_hybrid::WebGlRenderer,
    }

    impl std::fmt::Debug for Backend {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Backend(hybrid)").finish()
        }
    }

    impl Backend {
        pub fn new(canvas: &HtmlCanvasElement) -> Self {
            Self {
                renderer: vello_hybrid::WebGlRenderer::new(canvas),
            }
        }

        pub fn render(&mut self, ctx: &mut DrawContext) {
            let rs = vello_hybrid::RenderSize {
                width: ctx.width() as u32,
                height: ctx.height() as u32,
            };
            self.renderer.render(ctx, &rs).unwrap();
        }

        pub fn resize(&mut self, _w: u32, _h: u32) {}

        pub fn upload_image(&mut self, _ctx: &mut DrawContext, pixmap: Pixmap) -> ImageId {
            self.renderer.upload_image(&pixmap)
        }

        pub fn sync(&self) {
            crate::gpu_sync(&self.renderer);
        }
    }
}

pub use inner::*;

/// Create a new draw context with the given size.
pub fn new_draw_context(w: u32, h: u32) -> DrawContext {
    DrawContext::new(w as u16, h as u16)
}
