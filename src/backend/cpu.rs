use alloc::sync::Arc;

use vello_common::filter_effects::Filter;
use vello_common::glyph::Glyph;
use vello_common::kurbo::{Affine, BezPath, Rect, Stroke};
use vello_common::paint::{ImageSource, PaintType};
use vello_common::peniko::{Fill, FontData};
use vello_common::pixmap::Pixmap;
use wasm_bindgen::JsCast;
use web_sys::{
    HtmlCanvasElement, WebGl2RenderingContext as GL, WebGlBuffer, WebGlProgram, WebGlTexture,
    WebGlUniformLocation,
};

use crate::backend::{Backend, BackendKind, layout_text_glyphs};
use crate::capability::CapabilityProfile;
use crate::scenes::{ParamId, SceneId};

pub(crate) const CAPABILITIES: CapabilityProfile =
    CapabilityProfile::all().deny_params(SceneId::Rect, &[ParamId::UseDrawImage]);

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

pub struct BackendImpl {
    ctx: vello_cpu::RenderContext,
    width: u16,
    height: u16,
    gl: GL,
    program: WebGlProgram,
    quad_buffer: WebGlBuffer,
    position_loc: u32,
    sampler_loc: WebGlUniformLocation,
    texture: WebGlTexture,
    target: Pixmap,
}

impl std::fmt::Debug for BackendImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backend(cpu)").finish()
    }
}

impl BackendImpl {
    pub fn new(canvas: &HtmlCanvasElement, w: u32, h: u32) -> Self {
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
        let quad_buffer = gl.create_buffer().unwrap();
        gl.bind_buffer(GL::ARRAY_BUFFER, Some(&quad_buffer));
        let view = js_sys::Float32Array::new_with_length(8);
        view.copy_from(&verts);
        gl.buffer_data_with_array_buffer_view(GL::ARRAY_BUFFER, &view, GL::STATIC_DRAW);
        let position_loc = gl.get_attrib_location(&program, "p") as u32;
        gl.enable_vertex_attrib_array(position_loc);
        gl.vertex_attrib_pointer_with_i32(position_loc, 2, GL::FLOAT, false, 0, 0);

        let texture = gl.create_texture().unwrap();
        gl.active_texture(GL::TEXTURE0);
        gl.bind_texture(GL::TEXTURE_2D, Some(&texture));
        gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MIN_FILTER, GL::NEAREST as i32);
        gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MAG_FILTER, GL::NEAREST as i32);

        gl.use_program(Some(&program));
        let sampler_loc = gl
            .get_uniform_location(&program, "t")
            .expect("missing texture sampler uniform");
        gl.uniform1i(Some(&sampler_loc), 0);
        gl.disable(GL::BLEND);

        Self {
            ctx: vello_cpu::RenderContext::new(w as u16, h as u16),
            width: w as u16,
            height: h as u16,
            gl,
            program,
            quad_buffer,
            position_loc,
            sampler_loc,
            texture,
            target: Pixmap::new(w as u16, h as u16),
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
        BackendKind::Cpu
    }

    fn reset(&mut self) {
        self.ctx.reset();
    }

    fn render_offscreen(&mut self) {
        self.ctx.flush();
        self.ctx.render_to_pixmap(&mut self.target);
    }

    fn blit(&mut self) {
        let bytes: &[u8] = bytemuck::cast_slice(self.target.data());

        self.gl.use_program(Some(&self.program));
        self.gl
            .bind_buffer(GL::ARRAY_BUFFER, Some(&self.quad_buffer));
        self.gl.enable_vertex_attrib_array(self.position_loc);
        self.gl
            .vertex_attrib_pointer_with_i32(self.position_loc, 2, GL::FLOAT, false, 0, 0);
        self.gl.active_texture(GL::TEXTURE0);
        self.gl.bind_texture(GL::TEXTURE_2D, Some(&self.texture));
        self.gl.uniform1i(Some(&self.sampler_loc), 0);
        self.gl
            .tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
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

        self.gl
            .viewport(0, 0, self.width as i32, self.height as i32);
        self.gl.draw_arrays(GL::TRIANGLE_STRIP, 0, 4);
    }

    fn is_cpu(&self) -> bool {
        true
    }

    fn supports_encode_timing(&self) -> bool {
        true
    }

    fn resize(&mut self, w: u32, h: u32) {
        self.width = w as u16;
        self.height = h as u16;
        self.ctx = vello_cpu::RenderContext::new(w as u16, h as u16);
        self.target = Pixmap::new(self.width, self.height);
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
        ImageSource::Pixmap(Arc::new(pixmap))
    }

    fn destroy_image(&mut self, _image: &ImageSource) {}
}

impl Drop for BackendImpl {
    fn drop(&mut self) {
        self.gl.delete_texture(Some(&self.texture));
        self.gl.delete_buffer(Some(&self.quad_buffer));
        self.gl.delete_program(Some(&self.program));
    }
}
