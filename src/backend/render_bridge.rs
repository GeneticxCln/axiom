//! Transitional nested presentation bridge helpers.
//!
//! This module isolates the remaining WGPU → CPU readback → GL upload →
//! fullscreen blit path from the main backend event/state machine. It does not
//! change the architecture decision (WGPU-first), but it keeps the compatibility
//! presentation shim contained in one place.

use super::AxiomSmithayBackendReal;
use anyhow::Result;
use log::warn;
use std::sync::OnceLock;

fn ensure_gl_loaded() {
    static LOADED: OnceLock<()> = OnceLock::new();
    LOADED.get_or_init(|| {
        unsafe {
            gl::load_with(|name| {
                let name_c = std::ffi::CString::new(name).unwrap();
                let ptr = egl_get_proc_address(name_c.as_ptr());
                std::mem::transmute::<*const std::ffi::c_void, *const std::ffi::c_void>(ptr)
            });
        }
    });
}
fn egl_get_proc_address(name: *const i8) -> *const std::ffi::c_void {
    // Load libEGL at runtime (already loaded by smithay's EGL context)
    unsafe {
        static EGL_LIB: std::sync::OnceLock<libloading::Library> = std::sync::OnceLock::new();
        let lib = EGL_LIB.get_or_init(|| {
            libloading::Library::new("libEGL.so.1").expect("Failed to load libEGL.so.1")
        });
        let func: libloading::Symbol<extern "C" fn(*const i8) -> *const std::ffi::c_void> =
            lib.get(b"eglGetProcAddress").expect("eglGetProcAddress not found");
        func(name)
    }
}

impl AxiomSmithayBackendReal {
    /// Transitional nested presentation path: WGPU composes the full frame,
    /// then the composed image crosses an explicit compatibility bridge
    /// (CPU readback -> GL upload -> fullscreen blit).
    pub(super) fn compose_and_present_via_gl_bridge(
        &mut self,
        width: u32,
        height: u32,
        popup_ids: &[u32],
    ) -> Result<()> {
        let composed = if let Some(ref renderer) = self.state.renderer {
            renderer.write().compose_full_frame(width, height)
        } else {
            self.present_clear_gl()?;
            return Ok(());
        };

        match composed {
            Ok(composed) => {
                self.present_rgba_via_gl_bridge(width, height, &composed)?;
            }
            Err(e) => {
                warn!("⚠️ WGPU compose failed: {}", e);
                self.present_clear_gl()?;
            }
        }

        if let Some(ref renderer) = self.state.renderer {
            let mut r = renderer.write();
            for popup_id in popup_ids {
                r.remove_window(popup_render_id(*popup_id));
            }
        }

        Ok(())
    }

    /// Present a fully-composed RGBA frame through the temporary GL bridge.
    /// Currently a no-op (just binds and returns) — the raw GL blit code
    /// is temporarily disabled because NVIDIA's EGL implementation doesn't
    /// support the gl::* calls from a smithay-managed context. A future
    /// commit will restore this using smithay's GlesRenderer.
    pub(super) fn present_rgba_via_gl_bridge(
        &mut self,
        width: u32,
        height: u32,
        composed: &[u8],
    ) -> Result<()> {
        let Some(backend) = self.winit_backend.as_mut() else {
            return Ok(());
        };
        backend.bind()?;
        Ok(())
    }

    /// Clear the current GL framebuffer without going through the WGPU bridge.
    pub(super) fn present_clear_gl(&mut self) -> Result<()> {
        let Some(backend) = self.winit_backend.as_mut() else {
            return Ok(());
        };
        // Bind and submit without any GL calls — smithay's GlesRenderer
        // already set up the framebuffer. A bare submit is enough to
        // present the last rendered frame (or empty dark background).
        backend.bind()?;
        Ok(())
    }
}

/// Return `true` when the current frame has actual scene content that must
/// cross the temporary WGPU->GL presentation bridge.
pub(super) fn should_use_wgpu_gl_bridge(
    has_tiled_windows: bool,
    has_floating_windows: bool,
    committed_popup_count: usize,
) -> bool {
    has_tiled_windows || has_floating_windows || committed_popup_count > 0
}

/// Temporary render-ID namespace for popup surfaces staged into the WGPU
/// scene graph. Kept in a helper so the bridge path owns the convention in
/// one place.
pub(super) fn popup_render_id(popup_id: u32) -> u64 {
    0x8000_0000 + popup_id as u64
}

// ── GL Blit Helpers ────────────────────────────────────────────────────────
// Minimal GL code for the final fullscreen blit. The textured-quad shader is
// compiled once and cached in `AxiomSmithayBackendReal::blit_shader`.

/// Compile the minimal GLES 2.0 textured-quad shader used for the final
/// fullscreen blit. Returns the cached program on subsequent calls.
///
/// # Safety
/// Caller must ensure a GL context is current.
unsafe fn ensure_blit_shader(shader: &mut Option<gl::types::GLuint>) -> Option<gl::types::GLuint> {
    if let Some(prog) = *shader {
        return Some(prog);
    }

    let vert_src = r#"
        attribute vec2 a_position;
        attribute vec2 a_texcoord;
        varying vec2 v_texcoord;
        void main() {
            gl_Position = vec4(a_position, 0.0, 1.0);
            v_texcoord = a_texcoord;
        }
    "#;

    let frag_src = r#"
        precision mediump float;
        varying vec2 v_texcoord;
        uniform sampler2D u_texture;
        void main() {
            gl_FragColor = texture2D(u_texture, v_texcoord);
        }
    "#;

    unsafe fn compile_shader(ty: gl::types::GLenum, src: &str) -> Option<gl::types::GLuint> {
        let s = gl::CreateShader(ty);
        if s == 0 {
            return None;
        }
        gl::ShaderSource(
            s,
            1,
            &(src.as_ptr() as *const gl::types::GLchar),
            &(src.len() as gl::types::GLint),
        );
        gl::CompileShader(s);
        let mut ok: gl::types::GLint = 0;
        gl::GetShaderiv(s, gl::COMPILE_STATUS, &mut ok);
        if ok == 0 {
            gl::DeleteShader(s);
            return None;
        }
        Some(s)
    }

    let vs = compile_shader(gl::VERTEX_SHADER, vert_src);
    let fs = compile_shader(gl::FRAGMENT_SHADER, frag_src);

    if let (Some(vs), Some(fs)) = (vs, fs) {
        let prog = gl::CreateProgram();
        gl::AttachShader(prog, vs);
        gl::AttachShader(prog, fs);
        gl::LinkProgram(prog);
        let mut linked: gl::types::GLint = 0;
        gl::GetProgramiv(prog, gl::LINK_STATUS, &mut linked);
        if linked != 0 {
            *shader = Some(prog);
        }
        gl::DeleteShader(vs);
        gl::DeleteShader(fs);
        return *shader;
    }

    if let Some(v) = vs {
        gl::DeleteShader(v);
    }
    if let Some(f) = fs {
        gl::DeleteShader(f);
    }
    None
}

/// Upload RGBA data to a persistent GL texture, reusing it when dimensions match.
///
/// # Safety
/// Caller must ensure a GL context is current.
unsafe fn update_blit_texture(
    cache: &mut Option<(gl::types::GLuint, u32, u32)>,
    width: u32,
    height: u32,
    data: &[u8],
) -> gl::types::GLuint {
    if cache.is_none() {
        let mut t: gl::types::GLuint = 0;
        gl::GenTextures(1, &mut t);
        gl::BindTexture(gl::TEXTURE_2D, t);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
        gl::BindTexture(gl::TEXTURE_2D, 0);
        *cache = Some((t, 0, 0));
    }

    let (tex, old_w, old_h) = cache.as_mut().expect("just initialized");

    gl::BindTexture(gl::TEXTURE_2D, *tex);
    if *old_w == width && *old_h == height {
        gl::TexSubImage2D(
            gl::TEXTURE_2D,
            0,
            0,
            0,
            width as i32,
            height as i32,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            data.as_ptr() as *const std::ffi::c_void,
        );
    } else {
        gl::TexImage2D(
            gl::TEXTURE_2D,
            0,
            gl::RGBA as i32,
            width as i32,
            height as i32,
            0,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            data.as_ptr() as *const std::ffi::c_void,
        );
        *old_w = width;
        *old_h = height;
    }
    gl::BindTexture(gl::TEXTURE_2D, 0);
    *tex
}

/// Draw a fullscreen textured quad for the final blit.
///
/// # Safety
/// Caller must ensure a GL context is current and `tex` is a valid GL texture.
unsafe fn draw_blit_quad(shader: Option<gl::types::GLuint>, tex: gl::types::GLuint) {
    let Some(prog) = shader else {
        return;
    };

    #[rustfmt::skip]
    let vertices: [f32; 16] = [
        -1.0,  1.0, 0.0, 1.0,
         1.0,  1.0, 1.0, 1.0,
        -1.0, -1.0, 0.0, 0.0,
         1.0, -1.0, 1.0, 0.0,
    ];

    gl::UseProgram(prog);
    gl::ActiveTexture(gl::TEXTURE0);
    gl::BindTexture(gl::TEXTURE_2D, tex);

    let pos_loc = gl::GetAttribLocation(prog, c"a_position".as_ptr());
    let tex_loc = gl::GetAttribLocation(prog, c"a_texcoord".as_ptr());

    let stride = (4 * std::mem::size_of::<f32>()) as gl::types::GLsizei;

    if pos_loc >= 0 {
        gl::EnableVertexAttribArray(pos_loc as gl::types::GLuint);
        gl::VertexAttribPointer(
            pos_loc as gl::types::GLuint,
            2,
            gl::FLOAT,
            gl::FALSE,
            stride,
            vertices.as_ptr() as *const std::ffi::c_void,
        );
    }
    if tex_loc >= 0 {
        gl::EnableVertexAttribArray(tex_loc as gl::types::GLuint);
        gl::VertexAttribPointer(
            tex_loc as gl::types::GLuint,
            2,
            gl::FLOAT,
            gl::FALSE,
            stride,
            vertices.as_ptr().add(2) as *const std::ffi::c_void,
        );
    }

    gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);

    if pos_loc >= 0 {
        gl::DisableVertexAttribArray(pos_loc as gl::types::GLuint);
    }
    if tex_loc >= 0 {
        gl::DisableVertexAttribArray(tex_loc as gl::types::GLuint);
    }

    gl::BindTexture(gl::TEXTURE_2D, 0);
    gl::UseProgram(0);
}
