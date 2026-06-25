//! OpenGL texture upload pipeline for the Smithay backend.
//!
//! This module owns the SHM → GL upload glue. Wayland client SHM buffer
//! bytes arrive via the `wl_shm` protocol, are cached in
//! `State::buffer_cache`, then promoted into GPU textures via helpers in
//! this file. The shader compile/link path is also here so the dispatch
//! is concentrated in one place.
//!
//! Extracted from `src/backend/mod.rs` (formerly inlined inside
//! `AxiomSmithayBackendReal::render`). All helpers are static / take the
//! exact state slices they need — they do not borrow
//! `AxiomSmithayBackendReal`. This keeps the borrow checker happy when
//! the orchestrating struct immutably borrows the surface layout while
//! these helpers mutably borrow the render output cache.

use log::{debug, info, warn};
use std::collections::HashMap;

/// Per-frame GL texture upload entry — (surface_id, raw_rgba_data, (w, h)).
pub(super) type PendingTextureUpload = (u32, Vec<u8>, (i32, i32));

/// Parameters for drawing a textured GL quad in pixel space.
pub(super) struct TexQuadParams {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub screen_w: u32,
    pub screen_h: u32,
}

/// Check for OpenGL errors and log any found.
///
/// # Safety
/// A GL context must be current. `gl::GetError` is documented as safe to
/// call with any context state; this function only reads.
pub(super) fn gl_check_error(context: &str) {
    // SAFETY: GL context is current. GetError is always safe to call;
    // no memory is allocated or freed here.
    unsafe {
        loop {
            let err = gl::GetError();
            if err == gl::NO_ERROR {
                break;
            }
            let err_name = match err {
                gl::INVALID_ENUM => "GL_INVALID_ENUM",
                gl::INVALID_VALUE => "GL_INVALID_VALUE",
                gl::INVALID_OPERATION => "GL_INVALID_OPERATION",
                gl::OUT_OF_MEMORY => "GL_OUT_OF_MEMORY",
                other => {
                    warn!(
                        "⚠️ GL error ({}) in {}: code 0x{:X}",
                        context, context, other
                    );
                    continue;
                }
            };
            warn!("⚠️ GL error ({}) in {}", err_name, context);
        }
    }
}

/// Compile a single GLES 2.0 shader (`GL_VERTEX_SHADER` or
/// `GL_FRAGMENT_SHADER`) from inline GLSL source.
///
/// # Safety
/// Caller must ensure a GL context is current. Source pointer is borrowed
/// for the duration of the call only.
unsafe fn compile_shader(
    shader_type: gl::types::GLenum,
    source: &str,
) -> Option<gl::types::GLuint> {
    let shader = gl::CreateShader(shader_type);
    if shader == 0 {
        return None;
    }
    gl::ShaderSource(
        shader,
        1,
        &(source.as_ptr() as *const gl::types::GLchar),
        &(source.len() as gl::types::GLint),
    );
    gl::CompileShader(shader);
    let mut compiled: gl::types::GLint = 0;
    gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut compiled);
    if compiled == 0 {
        let mut len = 0;
        gl::GetShaderiv(shader, gl::INFO_LOG_LENGTH, &mut len);
        let mut buf = vec![0u8; len as usize];
        gl::GetShaderInfoLog(
            shader,
            len,
            std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut gl::types::GLchar,
        );
        warn!("Shader compile failed: {}", String::from_utf8_lossy(&buf));
        gl::DeleteShader(shader);
        return None;
    }
    Some(shader)
}

/// Compile + link the global textured-quad shader program on first use;
/// subsequent calls return the cached program.
///
/// # Safety
/// Caller must ensure a GL context is current.
pub(super) fn ensure_shader_program(
    shader_program: &mut Option<gl::types::GLuint>,
) -> Option<gl::types::GLuint> {
    if let Some(prog) = *shader_program {
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

    // SAFETY: GL context is current. CreateProgram/AttachShader/LinkProgram
    // operate on GL-owned objects; intermediate objects are deleted
    // regardless of link outcome.
    unsafe {
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
                info!(
                    "🎨 GLES 2.0 texture shader compiled successfully (program {})",
                    prog
                );
                *shader_program = Some(prog);
            } else {
                let mut len = 0;
                gl::GetProgramiv(prog, gl::INFO_LOG_LENGTH, &mut len);
                let mut buf = vec![0u8; len as usize];
                gl::GetProgramInfoLog(
                    prog,
                    len,
                    std::ptr::null_mut(),
                    buf.as_mut_ptr() as *mut gl::types::GLchar,
                );
                warn!("Shader link failed: {}", String::from_utf8_lossy(&buf));
                gl::DeleteProgram(prog);
            }

            gl::DeleteShader(vs);
            gl::DeleteShader(fs);
            return *shader_program;
        }

        if let Some(v) = vs {
            gl::DeleteShader(v);
        }
        if let Some(f) = fs {
            gl::DeleteShader(f);
        }
    }

    None
}

/// Upload raw RGBA SHM buffer data to an OpenGL texture for a surface.
/// Lazily generates the texture handle on first upload, then updates in
/// place. Returns the inserted GL texture handle on success.
///
/// # Safety
/// Caller must ensure a GL context is current, and `data.len()` must equal
/// `width * height * 4` bytes (RGBA8). `width` and `height` must be > 0.
pub(super) fn upload_gl_texture(
    texture_cache: &mut HashMap<u32, gl::types::GLuint>,
    surface_id: u32,
    data: &[u8],
    width: i32,
    height: i32,
) -> gl::types::GLuint {
    // SAFETY: see function-level safety contract.
    unsafe {
        let tex_id = texture_cache.get(&surface_id).copied().unwrap_or_else(|| {
            let mut tex: gl::types::GLuint = 0;
            gl::GenTextures(1, &mut tex);
            debug!("🖼️ Created GL texture {} for surface {}", tex, surface_id);
            tex
        });

        gl::BindTexture(gl::TEXTURE_2D, tex_id);
        gl::TexImage2D(
            gl::TEXTURE_2D,
            0,
            gl::RGBA as i32,
            width,
            height,
            0,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            data.as_ptr() as *const std::ffi::c_void,
        );
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
        gl::BindTexture(gl::TEXTURE_2D, 0);

        texture_cache.insert(surface_id, tex_id);
        tex_id
    }
}

/// Draw a textured quad in pixel coordinates using the cached shader
/// program. Pixel coords are converted to NDC inside the helper.
///
/// # Safety
/// Caller must ensure a GL context is current and `tex_id` is a valid GL
/// texture handle (e.g. from [`upload_gl_texture`]).
pub(super) fn draw_textured_quad(
    shader_program: Option<gl::types::GLuint>,
    tex_id: gl::types::GLuint,
    params: &TexQuadParams,
) {
    let Some(prog) = shader_program else {
        return;
    };

    let sw = params.screen_w as f32;
    let sh = params.screen_h as f32;
    let x1 = (params.x as f32 / sw) * 2.0 - 1.0;
    let y1 = (params.y as f32 / sh) * 2.0 - 1.0;
    let x2 = ((params.x + params.w) as f32 / sw) * 2.0 - 1.0;
    let y2 = ((params.y + params.h) as f32 / sh) * 2.0 - 1.0;

    #[rustfmt::skip]
    let vertices: [f32; 16] = [
        x1, y1, 0.0, 0.0,
        x2, y1, 1.0, 0.0,
        x1, y2, 0.0, 1.0,
        x2, y2, 1.0, 1.0,
    ];

    // SAFETY: GL context is current. UseProgram / BindTexture /
    // VertexAttribPointer all reference `vertices` (stack-allocated,
    // outlives the draw call) and `tex_id` (validated by caller).
    // Attrib arrays are disabled before exit.
    unsafe {
        gl::UseProgram(prog);
        gl::ActiveTexture(gl::TEXTURE0);
        gl::BindTexture(gl::TEXTURE_2D, tex_id);

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
}

/// Draw a fullscreen textured quad (covers the entire framebuffer).
///
/// Coordinates are GL-native (V=0 at bottom) since `glReadPixels` returns
/// bottom-to-top pixel order and matches GL's texture origin.
///
/// # Safety
/// Caller must ensure a GL context is current.
pub(super) fn draw_fullscreen_quad(
    shader_program: Option<gl::types::GLuint>,
    tex_id: gl::types::GLuint,
) {
    let Some(prog) = shader_program else {
        return;
    };

    #[rustfmt::skip]
    let vertices: [f32; 16] = [
        -1.0,  1.0, 0.0, 1.0,
         1.0,  1.0, 1.0, 1.0,
        -1.0, -1.0, 0.0, 0.0,
         1.0, -1.0, 1.0, 0.0,
    ];

    // SAFETY: GL context is current. `vertices` outlives the draw call.
    unsafe {
        gl::UseProgram(prog);
        gl::ActiveTexture(gl::TEXTURE0);
        gl::BindTexture(gl::TEXTURE_2D, tex_id);

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
}

/// Read the current GL framebuffer as RGBA pixels (bottom-to-top).
///
/// Caller must ensure the GL context is bound and the default
/// framebuffer is the active read target. Returns `width * height * 4`
/// bytes of RGBA data.
///
/// # Safety
/// GL context must be current and `pixels` (managed internally) must be
/// a unique, properly-aligned destination buffer.
pub(super) fn read_gl_framebuffer(width: u32, height: u32) -> Vec<u8> {
    let len = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(4);
    let mut pixels = vec![0u8; len];
    // SAFETY: GL context is current. PixelStorei + ReadPixels write into
    // `pixels` which is pre-allocated to exactly width*height*4 bytes.
    unsafe {
        gl::PixelStorei(gl::PACK_ALIGNMENT, 1);
        gl::ReadPixels(
            0,
            0,
            width as i32,
            height as i32,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            pixels.as_mut_ptr() as *mut std::ffi::c_void,
        );
    }
    gl_check_error("read_gl_framebuffer");
    pixels
}

/// Drain `buffer_cache` into a Vec of pending uploads. After this returns,
/// `buffer_cache` and `buffer_cache_dimensions` are clear.
///
/// `surface_id → texture_cache` mapping is preserved (entries present
/// before are skipped) so already-uploaded textures aren't reuploaded.
pub(super) fn collect_pending_uploads(
    buffer_cache: &mut HashMap<u32, Vec<u8>>,
    buffer_cache_dimensions: &mut HashMap<u32, (i32, i32)>,
    texture_cache: &HashMap<u32, gl::types::GLuint>,
) -> Vec<PendingTextureUpload> {
    let pending: Vec<PendingTextureUpload> = buffer_cache
        .iter()
        .filter(|(&sid, _)| !texture_cache.contains_key(&sid))
        .map(|(&sid, data)| {
            let dims = buffer_cache_dimensions
                .get(&sid)
                .copied()
                .unwrap_or((640, 480));
            (sid, data.clone(), dims)
        })
        .collect();
    buffer_cache.clear();
    buffer_cache_dimensions.clear();
    pending
}

/// Delete a list of stale GL texture handles that were queued for
/// deferred cleanup (from surfaces whose clients disconnected).
///
/// # Safety
/// Caller must ensure a GL context is current and all `handles` are valid
/// GL texture names.
pub(super) fn delete_textures(handles: &mut Vec<gl::types::GLuint>) {
    // SAFETY: GL context is current. Handles are valid GL texture names
    // generated by GenTextures earlier in the session.
    unsafe {
        for tex in handles.drain(..) {
            gl::DeleteTextures(1, &tex);
        }
    }
    gl_check_error("texture cleanup");
}
