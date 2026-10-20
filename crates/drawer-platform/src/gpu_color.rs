//! Native smoke compares actual RGBA32F shader output against the float64 CPU compositor.
use drawer_core::{
    color_precision::{Depth, Sample},
    layer_blend::Mode,
};
use glow::HasContext;
struct Objects<'a> {
    gl: &'a glow::Context,
    program: Option<glow::Program>,
    shaders: Vec<glow::Shader>,
    texture: Option<glow::Texture>,
    framebuffer: Option<glow::Framebuffer>,
    vao: Option<glow::VertexArray>,
    viewport: [i32; 4],
    flags: Vec<(u32, bool)>,
    stores: Vec<(u32, i32)>,
    bindings: [u32; 5],
    color_mask: [bool; 4],
}
impl Drop for Objects<'_> {
    fn drop(&mut self) {
        // SAFETY: These objects belong to the current creation context and are released before it returns.
        unsafe {
            let gl = self.gl;
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.use_program(None);
            gl.bind_vertex_array(None);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.viewport(
                self.viewport[0],
                self.viewport[1],
                self.viewport[2],
                self.viewport[3],
            );
            if let Some(v) = self.vao {
                gl.delete_vertex_array(v);
            }
            if let Some(f) = self.framebuffer {
                gl.delete_framebuffer(f);
            }
            if let Some(t) = self.texture {
                gl.delete_texture(t);
            }
            for s in &self.shaders {
                gl.delete_shader(*s);
            }
            if let Some(p) = self.program {
                gl.delete_program(p);
            }
            for (flag, on) in &self.flags {
                if *on {
                    gl.enable(*flag);
                } else {
                    gl.disable(*flag);
                }
            }
            for (key, value) in &self.stores {
                gl.pixel_store_i32(*key, *value);
            }
            gl.color_mask(
                self.color_mask[0],
                self.color_mask[1],
                self.color_mask[2],
                self.color_mask[3],
            );
            use std::num::NonZeroU32;
            gl.use_program(NonZeroU32::new(self.bindings[0]).map(glow::NativeProgram));
            gl.bind_vertex_array(NonZeroU32::new(self.bindings[1]).map(glow::NativeVertexArray));
            gl.bind_texture(
                glow::TEXTURE_2D,
                NonZeroU32::new(self.bindings[2]).map(glow::NativeTexture),
            );
            gl.bind_framebuffer(
                glow::DRAW_FRAMEBUFFER,
                NonZeroU32::new(self.bindings[3]).map(glow::NativeFramebuffer),
            );
            gl.bind_framebuffer(
                glow::READ_FRAMEBUFFER,
                NonZeroU32::new(self.bindings[4]).map(glow::NativeFramebuffer),
            );
        }
    }
}
pub fn verify(gl: &glow::Context) -> Result<(), String> {
    #[link(name = "opengl32")]
    unsafe extern "system" {
        fn wglGetCurrentContext() -> isize;
    }
    // SAFETY: wglGetCurrentContext takes no pointers and returns zero when this thread has no context.
    if unsafe { wglGetCurrentContext() } == 0 {
        return Err("GPU validation requires a current native OpenGL context".into());
    }
    // SAFETY: eframe owns the live context on this thread; all buffer bounds and pixel store state are set below.
    unsafe { verify_current(gl) }
}
unsafe fn verify_current(gl: &glow::Context) -> Result<(), String> {
    // SAFETY: All sizes, formats, shader sources and uniform vector lengths below are fixed and owned here.
    unsafe {
        if gl.get_parameter_i32(glow::PIXEL_PACK_BUFFER_BINDING) != 0
            || gl.get_parameter_i32(glow::PIXEL_UNPACK_BUFFER_BINDING) != 0
        {
            return Err("GPU validation needs an idle pixel transfer state".into());
        }
        let mut viewport = [0; 4];
        gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
        let mut objects = Objects {
            gl,
            program: None,
            shaders: vec![],
            texture: None,
            framebuffer: None,
            vao: None,
            viewport,
            flags: [
                glow::BLEND,
                glow::SCISSOR_TEST,
                glow::DEPTH_TEST,
                glow::STENCIL_TEST,
                glow::FRAMEBUFFER_SRGB,
                glow::DITHER,
            ]
            .map(|v| (v, gl.is_enabled(v)))
            .to_vec(),
            stores: [
                glow::PACK_ALIGNMENT,
                glow::PACK_SWAP_BYTES,
                glow::PACK_ROW_LENGTH,
                glow::PACK_SKIP_PIXELS,
                glow::PACK_SKIP_ROWS,
            ]
            .map(|v| (v, gl.get_parameter_i32(v)))
            .to_vec(),
            bindings: [
                glow::CURRENT_PROGRAM,
                glow::VERTEX_ARRAY_BINDING,
                glow::TEXTURE_BINDING_2D,
                glow::DRAW_FRAMEBUFFER_BINDING,
                glow::READ_FRAMEBUFFER_BINDING,
            ]
            .map(|v| gl.get_parameter_i32(v) as u32),
            color_mask: {
                let mut v = [0; 4];
                gl.get_parameter_i32_slice(glow::COLOR_WRITEMASK, &mut v);
                v.map(|v| v != 0)
            },
        };
        gl.pixel_store_i32(glow::PACK_ALIGNMENT, 4);
        gl.pixel_store_i32(glow::PACK_SWAP_BYTES, 0);
        gl.pixel_store_i32(glow::PACK_ROW_LENGTH, 0);
        gl.pixel_store_i32(glow::PACK_SKIP_PIXELS, 0);
        gl.pixel_store_i32(glow::PACK_SKIP_ROWS, 0);
        gl.color_mask(true, true, true, true);
        let program = gl.create_program()?;
        objects.program = Some(program);
        for (kind, source) in [
            (
                glow::VERTEX_SHADER,
                "#version 330 core\nvoid main(){vec2 p=vec2((gl_VertexID<<1)&2,gl_VertexID&2);gl_Position=vec4(p*2.-1.,0.,1.);}",
            ),
            (glow::FRAGMENT_SHADER, include_str!("gpu_color.frag")),
        ] {
            let shader = gl.create_shader(kind)?;
            objects.shaders.push(shader);
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                return Err(format!(
                    "Precision GPU shader: {}",
                    gl.get_shader_info_log(shader)
                ));
            }
            gl.attach_shader(program, shader);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            return Err(format!(
                "Precision GPU link: {}",
                gl.get_program_info_log(program)
            ));
        }
        let texture = gl.create_texture()?;
        objects.texture = Some(texture);
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA32F as i32,
            1,
            1,
            0,
            glow::RGBA,
            glow::FLOAT,
            glow::PixelUnpackData::Slice(None),
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::NEAREST as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::NEAREST as i32,
        );
        let framebuffer = gl.create_framebuffer()?;
        objects.framebuffer = Some(framebuffer);
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(texture),
            0,
        );
        if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
            return Err("RGBA32F GPU framebuffer unavailable".into());
        }
        let vao = gl.create_vertex_array()?;
        objects.vao = Some(vao);
        gl.bind_vertex_array(Some(vao));
        gl.use_program(Some(program));
        gl.viewport(0, 0, 1, 1);
        for flag in [
            glow::BLEND,
            glow::SCISSOR_TEST,
            glow::DEPTH_TEST,
            glow::STENCIL_TEST,
            glow::FRAMEBUFFER_SRGB,
            glow::DITHER,
        ] {
            gl.disable(flag);
        }
        let uniform = |name| {
            gl.get_uniform_location(program, name)
                .ok_or_else(|| format!("GPU uniform {name} missing"))
        };
        let ub = uniform("back_color")?;
        let us = uniform("source_color")?;
        let uo = uniform("opacity")?;
        let um = uniform("mode")?;
        let un = uniform("steps")?;
        let uq = uniform("quant_max")?;
        let patches = [
            ([0; 4], [0; 4]),
            ([65535; 4], [0, 65535, 1, 32768]),
            ([1, 32768, 65534, 1], [65534, 1, 32768, 123]),
            ([12345, 23456, 34567, 45678], [54321, 43210, 32109, 32768]),
            ([32768; 4], [32768; 4]),
            ([0; 4], [12345, 23456, 34567, 1]),
        ];
        for (mode_index, mode) in Mode::ALL.into_iter().enumerate() {
            for (b, s) in patches {
                for opacity in [0., 0.37, 0.5, 1.] {
                    compare(
                        gl,
                        &[&ub, &us, &uo, &um, &un, &uq],
                        Sample::from16(b),
                        Sample::from16(s),
                        mode,
                        mode_index as i32,
                        opacity,
                        1,
                    )?;
                }
            }
        }
        for alpha in [0.00001, 0.5, 1.] {
            compare(
                gl,
                &[&ub, &us, &uo, &um, &un, &uq],
                Sample::default(),
                Sample::new([0.123456, 0.456789, 0.789012, alpha])?,
                Mode::Normal,
                0,
                1.,
                1000,
            )?;
        }
        for depth in [Depth::Eight, Depth::Sixteen] {
            for delta in [-1e-5, 0., 1e-5] {
                let alpha = 0.5 / depth.maximum() as f64 * (1. + delta);
                compare(
                    gl,
                    &[&ub, &us, &uo, &um, &un, &uq],
                    Sample::default(),
                    Sample::new([0.123456, 0.456789, 0.789012, alpha])?,
                    Mode::Normal,
                    0,
                    1.,
                    1,
                )?;
            }
        }
        let error = gl.get_error();
        if error != glow::NO_ERROR {
            return Err(format!("Precision GPU error: {error:#x}"));
        }
        Ok(())
    }
}
#[allow(clippy::too_many_arguments)]
unsafe fn compare(
    gl: &glow::Context,
    u: &[&glow::UniformLocation; 6],
    back: Sample,
    source: Sample,
    mode: Mode,
    mode_index: i32,
    opacity: f64,
    steps: i32,
) -> Result<(), String> {
    // SAFETY: The current context and allocated RGBA32F target are owned by the caller; readback is exactly 16 bytes.
    unsafe {
        let b = back.components();
        let s = source.components();
        gl.uniform_4_f32(
            Some(u[0]),
            b[0] as f32,
            b[1] as f32,
            b[2] as f32,
            b[3] as f32,
        );
        gl.uniform_4_f32(
            Some(u[1]),
            s[0] as f32,
            s[1] as f32,
            s[2] as f32,
            s[3] as f32,
        );
        gl.uniform_1_f32(Some(u[2]), opacity as f32);
        gl.uniform_1_i32(Some(u[3]), mode_index);
        gl.uniform_1_i32(Some(u[4]), steps);
        let mut reference = back;
        for _ in 0..steps {
            reference = reference.over(source, mode, opacity)?.clamped();
        }
        for depth in [Depth::Eight, Depth::Sixteen] {
            gl.uniform_1_f32(Some(u[5]), depth.maximum() as f32);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            let mut bytes = [0; 16];
            gl.read_pixels(
                0,
                0,
                1,
                1,
                glow::RGBA,
                glow::FLOAT,
                glow::PixelPackData::Slice(Some(&mut bytes)),
            );
            let values: [f32; 4] = std::array::from_fn(|n| {
                f32::from_ne_bytes(bytes[n * 4..n * 4 + 4].try_into().unwrap())
            });
            if values.iter().any(|v| {
                !v.is_finite() || *v < 0. || *v > depth.maximum() as f32 || v.fract() != 0.
            }) {
                return Err(format!("Invalid GPU integer readback: {values:?}"));
            }
            let output = values.map(|v| v as u16);
            let expected = reference.store(depth, Default::default(), [0; 2]);
            if output
                .into_iter()
                .zip(expected)
                .any(|(a, b)| a.abs_diff(b) > 1)
            {
                return Err(format!(
                    "GPU {} / {opacity} / {steps} / {depth:?}: {output:?}, float64 {expected:?}",
                    mode.label()
                ));
            }
        }
        Ok(())
    }
}
