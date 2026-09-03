// Drawing, on a context we own.
//
// SDL's 2D renderer went away with this file arriving, and the reason is not
// that it was slow. It creates its *own* GL context and gives no say in what
// version — on macOS a legacy 2.1 one, on Linux and Android a GLES 2.0 one —
// so a shader had to be written in whichever dialect it happened to hand back,
// and multi-pass work had to be interleaved with a batcher that does not know
// about it. The glass is render-to-texture and a blur, and that is not a thing
// to do through somebody else's batching.
//
// So: one context, asked for outright, GL 3.3 core on a desktop and GLES 3.0
// on the handheld. Both speak the same GLSL, so there is one dialect again.
// What is left is a couple of hundred lines that draw a rectangle with an
// optional texture on it, which is everything an interface of cards and words
// actually needs.

use anyhow::{Context, Result, anyhow};
use moose_rack::layout::Rect;
use std::ffi::CString;

/// The vertex shader: a quad in points, and where on a texture each corner
/// sits.
///
/// Positions arrive in the same points everything else is written in and are
/// turned into clip space here, so nothing above this file ever computes a
/// coordinate between -1 and 1.
// Locations are stated rather than left to the linker — see `glass.rs` for what
// happens when they are not.
const VERTEX: &str = r#"
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec2 a_uv;
out vec2 v_uv;
/// Where this pixel is inside its own quad, 0 to 1 in each direction.
///
/// Not `a_uv`: that is where the pixel is inside the *texture*, and the two
/// part company the moment anything draws a piece of a texture rather than
/// all of it. The corner rounding is about the quad's shape, so it gets a
/// coordinate of its own.
out vec2 v_quad;
uniform vec2 u_screen;
/// The quad, in pixels: where it is and how big.
uniform vec4 u_rect;
void main() {
  v_uv = a_uv;
  v_quad = (a_pos - u_rect.xy) / max(u_rect.zw, vec2(1.0));
  vec2 clip = vec2(a_pos.x / u_screen.x * 2.0 - 1.0, 1.0 - a_pos.y / u_screen.y * 2.0);
  gl_Position = vec4(clip, 0.0, 1.0);
}
"#;

/// The fragment shader: a color, optionally multiplied by a texture.
///
/// One program rather than two. Text is a coverage mask tinted at draw time
/// and artwork is a picture drawn as it is, and both are "a color times a
/// sample" — with a 1x1 white texture standing in when there is no picture, so
/// a plain rectangle takes the same path as everything else.
/// The fragment shader: a color, optionally multiplied by a texture, inside
/// rounded corners.
///
/// One program rather than two. Text is a coverage mask tinted at draw time
/// and artwork is a picture drawn as it is, and both are "a color times a
/// sample" — with a 1x1 white texture standing in when there is no picture, so
/// a plain rectangle takes the same path as everything else.
///
/// The corners are a signed distance to a rounded box, smoothed over one pixel.
/// `border-radius` is on nearly everything in `ui/style.css` and its absence is
/// most of why the SDL front end read as a prototype: square corners are what a
/// debug view looks like.
const FRAGMENT: &str = r#"
in vec2 v_uv;
in vec2 v_quad;
out vec4 color;
uniform sampler2D u_texture;
uniform vec4 u_tint;
/// The tint at the bottom edge. Equal to `u_tint` for a flat fill; different
/// for glass, which is lit from above the way a real sheet is — a flat tint
/// over a blur reads as a gray slab, which is what the panels were.
uniform vec4 u_tint2;
/// Half the quad, in pixels, and how round its corners are. Zero radius is a
/// plain rectangle and costs one comparison.
uniform vec2 u_half;
uniform float u_radius;

float rounded(vec2 p, vec2 half_size, float r) {
  vec2 q = abs(p) - (half_size - vec2(r));
  return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

void main() {
  vec4 texel = texture(u_texture, v_uv) * mix(u_tint, u_tint2, v_quad.y);
  if (u_radius > 0.0) {
    vec2 p = (v_quad - 0.5) * 2.0 * u_half;
    // Smoothed across one pixel, which is what stops a rounded corner looking
    // like a staircase.
    texel.a *= 1.0 - smoothstep(-1.0, 1.0, rounded(p, u_half, u_radius));
  }
  color = texel;
}
"#;

/// How many quads the vertex ring holds.
///
/// One frame of this interface is a few hundred: every rounded corner, every
/// word, every cover. A ring this size means a slot is reused only after
/// several hundred other draws have gone through, by which time the GPU is
/// long done with it.
const RING_QUADS: usize = 1024;

/// Somewhere to draw that is not the screen.
///
/// The glass needs one: the backdrop goes into a texture, the texture is
/// blurred, and the panels sample the result. None of that is possible while
/// drawing straight at the window.
pub struct Offscreen {
    frame: u32,
    pub texture: Texture,
}

impl Offscreen {
    /// # Safety
    ///
    /// A context must be current.
    pub unsafe fn new(width: u32, height: u32) -> Result<Self> {
        unsafe {
            let texture = upload_empty(width.max(1), height.max(1));
            let mut frame = 0;
            gl::GenFramebuffers(1, &mut frame);
            gl::BindFramebuffer(gl::FRAMEBUFFER, frame);
            gl::FramebufferTexture2D(
                gl::FRAMEBUFFER,
                gl::COLOR_ATTACHMENT0,
                gl::TEXTURE_2D,
                texture.id,
                0,
            );
            let ok = gl::CheckFramebufferStatus(gl::FRAMEBUFFER) == gl::FRAMEBUFFER_COMPLETE;
            gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
            if !ok {
                return Err(anyhow!(
                    "this driver will not draw into a texture that size"
                ));
            }
            Ok(Offscreen { frame, texture })
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.texture.width, self.texture.height)
    }

    /// The framebuffer name, for reading the pixels back out of it.
    pub fn frame_id(&self) -> u32 {
        self.frame
    }
}

impl Drop for Offscreen {
    fn drop(&mut self) {
        unsafe { gl::DeleteFramebuffers(1, &self.frame) }
    }
}

/// A color, 0 to 1.
#[derive(Debug, Clone, Copy)]
pub struct Rgba(pub f32, pub f32, pub f32, pub f32);

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Rgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
    }
    pub const WHITE: Rgba = Rgba(1.0, 1.0, 1.0, 1.0);
}

/// An image on the card. Deletes itself.
pub struct Texture {
    id: u32,
    /// Its own size in pixels. Not what it is drawn at — a cover is stretched
    /// to the card — but what it would be at one to one.
    pub width: u32,
    pub height: u32,
}

impl Drop for Texture {
    fn drop(&mut self) {
        unsafe { gl::DeleteTextures(1, &self.id) }
    }
}

pub struct Gfx {
    program: u32,
    vao: u32,
    vbo: u32,
    /// Which slot in the vertex ring the next quad goes into.
    slot: std::cell::Cell<usize>,
    u_screen: i32,
    u_tint: i32,
    u_tint2: i32,
    u_half: i32,
    u_radius: i32,
    u_rect: i32,
    /// Where the drawing area starts in the window.
    ///
    /// Non-zero when the panel is letterboxed. Kept because `draw_onto` has to
    /// put it back: restoring with the size alone left the viewport at the
    /// window's origin, and everything drawn after a glass capture — which is
    /// everything — landed offset from the panel it belonged to.
    origin: (f32, f32),
    /// The bottom tint for the next quad, when it is a gradient. Set around a
    /// draw like `radius`, so the call sites that want a flat fill say nothing.
    fade: std::cell::Cell<Option<Rgba>>,
    /// How round the next quad's corners are, in pixels. Set around a draw
    /// rather than passed to it, so the twenty call sites that want square
    /// corners say nothing.
    radius: std::cell::Cell<f32>,
    /// One white pixel, so a rectangle with no picture on it is the same draw
    /// as one with.
    blank: Texture,
    width: f32,
    height: f32,
}

impl Gfx {
    /// # Safety
    ///
    /// A context must be current, and must stay current for the life of this.
    pub unsafe fn new(video: &sdl2::VideoSubsystem) -> Result<Self> {
        unsafe {
            gl::load_with(|name| video.gl_get_proc_address(name) as *const _);
            let version = version_line().context("this machine's OpenGL")?;
            let program = link(
                &format!("{version}\n{}{VERTEX}", precision(version)),
                &format!("{version}\n{}{FRAGMENT}", precision(version)),
            )?;

            let (mut vao, mut vbo) = (0, 0);
            gl::GenVertexArrays(1, &mut vao);
            gl::GenBuffers(1, &mut vbo);
            gl::BindVertexArray(vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            // Four floats a vertex — two of position, two of texture — six
            // vertices a quad, and room for a frame's worth of quads laid end
            // to end. Filled per draw; there is no static geometry in an
            // interface where everything moves.
            //
            // A frame's worth rather than one, because writing over the same
            // twenty-four floats for every quad means the driver either waits
            // for the last draw to finish reading them or quietly allocates a
            // fresh buffer behind your back. On a Metal-backed GL that showed
            // up as the single most expensive thing in the app — more than the
            // backdrop shader and the blur put together.
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (RING_QUADS * 24 * 4) as isize,
                std::ptr::null(),
                gl::STREAM_DRAW,
            );
            bind_attribute(program, "a_pos", 0)?;
            bind_attribute(program, "a_uv", 2)?;
            gl::BindVertexArray(0);

            gl::Disable(gl::DEPTH_TEST);

            let blank = upload(1, 1, &[255, 255, 255, 255]);
            Ok(Gfx {
                origin: (0.0, 0.0),
                fade: std::cell::Cell::new(None),
                u_screen: uniform(program, "u_screen"),
                u_tint: uniform(program, "u_tint"),
                u_tint2: uniform(program, "u_tint2"),
                u_half: uniform(program, "u_half"),
                u_radius: uniform(program, "u_radius"),
                u_rect: uniform(program, "u_rect"),
                radius: std::cell::Cell::new(0.0),
                program,
                vao,
                vbo,
                slot: std::cell::Cell::new(0),
                blank,
                width: 1.0,
                height: 1.0,
            })
        }
    }

    /// Tell it how big the drawable is, in pixels.
    pub fn resize(&mut self, width: f32, height: f32) {
        self.resize_at(0.0, 0.0, width, height)
    }

    /// Draw into part of the window rather than all of it.
    ///
    /// The panel preview needs this: the layout is exactly 640x480 points and
    /// the window is whatever somebody dragged it to, so the panel is drawn at
    /// a whole-number scale in the middle and the rest of the window is left
    /// alone. Stretching to fill instead is how a 2784-pixel window came out
    /// 696 points wide and stopped being a preview of the device at all.
    pub fn resize_at(&mut self, x: f32, y: f32, width: f32, height: f32) {
        self.origin = (x, y);
        self.width = width.max(1.0);
        self.height = height.max(1.0);
        unsafe { gl::Viewport(x as i32, y as i32, self.width as i32, self.height as i32) };
    }

    pub fn clear(&self, color: Rgba) {
        unsafe {
            gl::ClearColor(color.0, color.1, color.2, color.3);
            gl::Clear(gl::COLOR_BUFFER_BIT);
        }
    }

    /// Round the corners of whatever is drawn inside `body`, by `pixels`.
    ///
    /// Scoped rather than a parameter, because a card is a picture inside a
    /// panel inside a column and all three want the same corner — and because
    /// every other call site wants none and should not have to say so.
    /// Draw the next quads with their tint fading to `bottom` down the quad.
    ///
    /// Set around a draw rather than passed to it, the same way `rounded` is, so
    /// the call sites that want a flat fill say nothing.
    pub fn faded<R>(&self, bottom: Rgba, body: impl FnOnce() -> R) -> R {
        let was = self.fade.replace(Some(bottom));
        let out = body();
        self.fade.set(was);
        out
    }

    pub fn rounded(&self, pixels: f32, body: impl FnOnce()) {
        let was = self.radius.get();
        self.radius.set(pixels.max(0.0));
        body();
        self.radius.set(was);
    }

    /// A filled rectangle, in pixels from the top left.
    pub fn rect(&self, x: f32, y: f32, w: f32, h: f32, color: Rgba) {
        self.quad(&self.blank, x, y, w, h, color);
    }

    /// The same, taking a box the layout worked out.
    ///
    /// Everything a view draws should come through one of these rather than
    /// four numbers it added up itself.
    pub fn fill(&self, r: Rect, color: Rgba) {
        self.quad(&self.blank, r.x, r.y, r.w, r.h, color);
    }

    /// A picture, filling the box and keeping its own shape.
    pub fn picture(&self, texture: &Texture, r: Rect, tint: Rgba) {
        let fitted = r.fit(texture.width.max(1) as f32 / texture.height.max(1) as f32);
        self.quad(texture, fitted.x, fitted.y, fitted.w, fitted.h, tint);
    }

    /// A border, as four filled edges — a cursor has to be visible against
    /// artwork, and a one-pixel line is not.
    pub fn outline(&self, r: Rect, thickness: f32, color: Rgba) {
        let t = thickness.min(r.w / 2.0).min(r.h / 2.0);
        self.fill(Rect::new(r.x, r.y, r.w, t), color);
        self.fill(Rect::new(r.x, r.bottom() - t, r.w, t), color);
        self.fill(Rect::new(r.x, r.y, t, r.h), color);
        self.fill(Rect::new(r.right() - t, r.y, t, r.h), color);
    }

    /// An image, stretched to fit.
    ///
    /// `tint` multiplies it, which is what makes one text raster serve every
    /// color it is drawn in — white with the coverage as alpha, tinted here.
    pub fn image(&self, texture: &Texture, x: f32, y: f32, w: f32, h: f32, tint: Rgba) {
        self.quad(texture, x, y, w, h, tint);
    }

    fn quad(&self, texture: &Texture, x: f32, y: f32, w: f32, h: f32, tint: Rgba) {
        self.quad_uv(texture, x, y, w, h, (0.0, 0.0, 1.0, 1.0), tint);
    }

    #[allow(clippy::too_many_arguments)]
    fn quad_uv(
        &self,
        texture: &Texture,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        (u0, v0, u1, v1): (f32, f32, f32, f32),
        tint: Rgba,
    ) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let (l, t, r, b) = (x, y, x + w, y + h);
        #[rustfmt::skip]
        let vertices: [f32; 24] = [
            l, t, u0, v0,
            r, t, u1, v0,
            l, b, u0, v1,
            l, b, u0, v1,
            r, t, u1, v0,
            r, b, u1, v1,
        ];
        unsafe {
            // Set here rather than once at startup. Anything else drawing on
            // this context — the backdrop turns blending off for its own quad,
            // which is opaque and covers the frame — leaves the state it
            // wanted behind, and a coverage mask drawn without blending is a
            // solid rectangle the color of its tint. Which is what every
            // label in the library became.
            gl::Enable(gl::BLEND);
            gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
            gl::UseProgram(self.program);
            gl::Uniform2f(self.u_screen, self.width, self.height);
            gl::Uniform4f(self.u_tint, tint.0, tint.1, tint.2, tint.3);
            let bottom = self.fade.get().unwrap_or(tint);
            gl::Uniform4f(self.u_tint2, bottom.0, bottom.1, bottom.2, bottom.3);
            gl::Uniform2f(self.u_half, w / 2.0, h / 2.0);
            gl::Uniform4f(self.u_rect, x, y, w, h);
            // Never more than half the shorter side, or the corners meet in
            // the middle and the quad turns into a lozenge.
            gl::Uniform1f(self.u_radius, self.radius.get().min(w.min(h) / 2.0));
            gl::ActiveTexture(gl::TEXTURE0);
            gl::BindTexture(gl::TEXTURE_2D, texture.id);
            gl::BindVertexArray(self.vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo);
            // Somewhere the last few draws are not still reading.
            let slot = self.slot.get();
            self.slot.set((slot + 1) % RING_QUADS);
            gl::BufferSubData(
                gl::ARRAY_BUFFER,
                (slot * 24 * 4) as isize,
                size_of_val(&vertices) as isize,
                vertices.as_ptr() as *const _,
            );
            gl::DrawArrays(gl::TRIANGLES, (slot * 6) as i32, 6);
            gl::BindVertexArray(0);
        }
    }

    /// Upload a picture: four bytes a pixel, top row first.
    pub fn upload_rgba(&self, width: u32, height: u32, pixels: &[u8]) -> Texture {
        unsafe { upload(width, height, pixels) }
    }

    /// Upload a coverage mask as white with that coverage for alpha.
    ///
    /// What text is. Kept white here so the color is decided at draw time and
    /// one raster serves a name however it is drawn — dim in a metadata line,
    /// bright under the cursor.
    pub fn upload_coverage(&self, width: u32, height: u32, coverage: &[u8]) -> Texture {
        let mut rgba = Vec::with_capacity(coverage.len() * 4);
        for &value in coverage {
            rgba.extend_from_slice(&[255, 255, 255, value]);
        }
        unsafe { upload(width, height, &rgba) }
    }
}

impl Gfx {
    /// Draw onto `target` instead of the window, for as long as the closure
    /// runs.
    ///
    /// The viewport goes with it and comes back: a texture is rarely the size
    /// of the screen, and a viewport left describing the wrong one draws the
    /// next frame into a corner.
    ///
    /// # Safety
    ///
    /// A context must be current.
    pub unsafe fn draw_onto(&mut self, target: &Offscreen, body: impl FnOnce(&mut Gfx)) {
        let (was_w, was_h) = (self.width, self.height);
        let (ox, oy) = self.origin;
        let (w, h) = target.size();
        unsafe {
            gl::BindFramebuffer(gl::FRAMEBUFFER, target.frame);
        }
        self.resize(w as f32, h as f32);
        body(self);
        unsafe {
            gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
        }
        // Back to where the panel is, not to the window's corner.
        self.resize_at(ox, oy, was_w, was_h);
    }

    /// Draw part of a texture, rather than all of it.
    ///
    /// What the glass needs: a panel shows the blurred backdrop from *behind
    /// itself*, so it samples the rectangle it occupies and no other.
    /// `source` is in texture coordinates, 0 to 1, top-left origin.
    #[allow(clippy::too_many_arguments)]
    pub fn image_part(
        &self,
        texture: &Texture,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        source: (f32, f32, f32, f32),
        tint: Rgba,
    ) {
        self.quad_uv(texture, x, y, w, h, source, tint);
    }
}

impl Drop for Gfx {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteProgram(self.program);
            gl::DeleteBuffers(1, &self.vbo);
            gl::DeleteVertexArrays(1, &self.vao);
        }
    }
}

/// A texture with nothing in it yet, to be drawn into.
unsafe fn upload_empty(width: u32, height: u32) -> Texture {
    unsafe { upload_raw(width, height, std::ptr::null()) }
}

impl Texture {
    /// Magnify this texture by repeating pixels rather than blending them.
    ///
    /// For the panel preview, and only for it. The device draws 640x480 real
    /// pixels; showing that four times bigger with linear filtering invents
    /// smooth edges the handheld will never have, which is exactly the thing
    /// the preview is supposed to be honest about.
    pub fn nearest(&self) {
        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, self.id);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
            gl::BindTexture(gl::TEXTURE_2D, 0);
        }
    }
}

unsafe fn upload(width: u32, height: u32, pixels: &[u8]) -> Texture {
    unsafe { upload_raw(width, height, pixels.as_ptr() as *const _) }
}

unsafe fn upload_raw(width: u32, height: u32, pixels: *const std::ffi::c_void) -> Texture {
    unsafe {
        let mut id = 0;
        gl::GenTextures(1, &mut id);
        gl::BindTexture(gl::TEXTURE_2D, id);
        // Linear, because a cover is drawn at whatever size the zoom asks for
        // and nearest makes that look like a screenshot of a screenshot.
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
        // Clamped, or the edge of a card samples the opposite edge of its own
        // artwork and every cover gets a thin wrong-colored line round it.
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
        // Rows are not padded. The default is four-byte alignment, which
        // shears any image whose width is not a multiple of four — and a text
        // raster is exactly as wide as the word in it.
        gl::PixelStorei(gl::UNPACK_ALIGNMENT, 1);
        gl::TexImage2D(
            gl::TEXTURE_2D,
            0,
            gl::RGBA as i32,
            width as i32,
            height as i32,
            0,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            pixels,
        );
        gl::BindTexture(gl::TEXTURE_2D, 0);
        Texture { id, width, height }
    }
}

unsafe fn bind_attribute(program: u32, name: &str, offset: usize) -> Result<()> {
    unsafe {
        let c = CString::new(name)?;
        let at = gl::GetAttribLocation(program, c.as_ptr());
        if at < 0 {
            return Err(anyhow!("the shader has no {name}"));
        }
        gl::EnableVertexAttribArray(at as u32);
        gl::VertexAttribPointer(
            at as u32,
            2,
            gl::FLOAT,
            gl::FALSE,
            (4 * std::mem::size_of::<f32>()) as i32,
            (offset * std::mem::size_of::<f32>()) as *const _,
        );
        Ok(())
    }
}

unsafe fn uniform(program: u32, name: &str) -> i32 {
    unsafe {
        let c = CString::new(name).unwrap();
        gl::GetUniformLocation(program, c.as_ptr())
    }
}

/// GLSL ES needs to be told a precision; desktop GLSL does not have the
/// statement at all before 1.30 and ignores it after.
fn precision(version: &str) -> &'static str {
    if version.contains(" es") {
        "precision highp float;\n"
    } else {
        ""
    }
}

/// Which GLSL this context speaks.
///
/// One answer now, not two: the context is ours, so it is the one we asked
/// for. A machine that cannot give it says so here rather than failing inside
/// a shader compiler.
pub fn version_line() -> Result<&'static str> {
    let reported = unsafe { reported(gl::SHADING_LANGUAGE_VERSION) };
    let es = reported.contains("ES");
    let number: f32 = reported
        .split_whitespace()
        .find_map(|word| word.parse().ok())
        .unwrap_or(0.0);
    match (es, number) {
        (true, n) if n >= 3.0 => Ok("#version 300 es"),
        (false, n) if n >= 3.3 => Ok("#version 330 core"),
        _ => Err(anyhow!(
            "reports GLSL {reported:?}; this needs GLSL 3.30 or GLSL ES 3.00"
        )),
    }
}

impl Texture {
    /// The name GL knows it by, for the passes that bind it themselves.
    pub fn raw(&self) -> u32 {
        self.id
    }
}

/// Draw a quad covering the whole target, with no program of our own bound.
///
/// For the shader passes that supply their own — the blur, and anything after
/// it. The vertices are in clip space, so nothing has to know how big the
/// target is.
///
/// # Safety
///
/// A context must be current, and a program bound.
pub unsafe fn draw_full_quad() {
    unsafe {
        static mut QUAD: (u32, u32) = (0, 0);
        if QUAD.0 == 0 {
            let corners: [f32; 24] = [
                -1.0, -1.0, 0.0, 0.0, 1.0, -1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 1.0, -1.0, 1.0, 0.0,
                1.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0,
            ];
            let (mut vao, mut vbo) = (0, 0);
            gl::GenVertexArrays(1, &mut vao);
            gl::GenBuffers(1, &mut vbo);
            gl::BindVertexArray(vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                size_of_val(&corners) as isize,
                corners.as_ptr() as *const _,
                gl::STATIC_DRAW,
            );
            for (index, offset) in [(0u32, 0usize), (1, 2)] {
                gl::EnableVertexAttribArray(index);
                gl::VertexAttribPointer(
                    index,
                    2,
                    gl::FLOAT,
                    gl::FALSE,
                    (4 * std::mem::size_of::<f32>()) as i32,
                    (offset * std::mem::size_of::<f32>()) as *const _,
                );
            }
            gl::BindVertexArray(0);
            QUAD = (vao, vbo);
        }
        gl::BindVertexArray(QUAD.0);
        gl::DrawArrays(gl::TRIANGLES, 0, 6);
        gl::BindVertexArray(0);
    }
}

/// One of GL's own strings — the version, the renderer, the shading language.
///
/// # Safety
///
/// A context must be current.
pub unsafe fn reported(name: u32) -> String {
    unsafe {
        let raw = gl::GetString(name);
        if raw.is_null() {
            return String::new();
        }
        std::ffi::CStr::from_ptr(raw as *const _)
            .to_string_lossy()
            .into_owned()
    }
}

/// Compile and link a program, reporting the whole log if it will not.
///
/// # Safety
///
/// A context must be current.
pub unsafe fn link(vertex: &str, fragment: &str) -> Result<u32> {
    unsafe {
        let v = compile(gl::VERTEX_SHADER, vertex)?;
        let f = compile(gl::FRAGMENT_SHADER, fragment)?;
        let program = gl::CreateProgram();
        gl::AttachShader(program, v);
        gl::AttachShader(program, f);
        gl::LinkProgram(program);
        gl::DeleteShader(v);
        gl::DeleteShader(f);
        let mut ok = 0;
        gl::GetProgramiv(program, gl::LINK_STATUS, &mut ok);
        if ok == 0 {
            return Err(anyhow!("linking: {}", info_log(program, true)));
        }
        Ok(program)
    }
}

unsafe fn compile(kind: u32, source: &str) -> Result<u32> {
    unsafe {
        let shader = gl::CreateShader(kind);
        let c = CString::new(source)?;
        gl::ShaderSource(shader, 1, &c.as_ptr(), std::ptr::null());
        gl::CompileShader(shader);
        let mut ok = 0;
        gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut ok);
        if ok == 0 {
            // The whole log. A shader that will not compile on a device we do
            // not have is a message somebody reads off a photograph.
            return Err(anyhow!("compiling: {}", info_log(shader, false)));
        }
        Ok(shader)
    }
}

unsafe fn info_log(object: u32, program: bool) -> String {
    unsafe {
        let mut len = 0;
        if program {
            gl::GetProgramiv(object, gl::INFO_LOG_LENGTH, &mut len);
        } else {
            gl::GetShaderiv(object, gl::INFO_LOG_LENGTH, &mut len);
        }
        let mut buf = vec![0u8; len.max(1) as usize];
        if program {
            gl::GetProgramInfoLog(
                object,
                len,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut _,
            );
        } else {
            gl::GetShaderInfoLog(
                object,
                len,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut _,
            );
        }
        String::from_utf8_lossy(&buf)
            .trim_end_matches('\0')
            .trim()
            .to_owned()
    }
}
