// The frosted panels.
//
// `ui/style.css` uses `backdrop-filter: blur(10px) saturate(150%)` in a
// hundred and thirty-one places, and it is the app's whole look — a card is a
// pane of frosted glass with the shader backdrop moving behind it.
//
// A webview gets that for one line of CSS. Here it is what the doc calls "the
// expensive one": the effect samples what is *behind* the element, so it needs
// the backdrop drawn into a texture, that texture blurred, and each panel
// drawing the part of the result that sits behind itself. Owning the GL
// context is what makes any of it possible; SDL's renderer had no way to draw
// into a texture at all.
//
// Two passes, horizontally then vertically. A proper two-dimensional blur of
// radius r samples r² texels per pixel; separated into two one-dimensional
// passes it is 2r, and for the radius this wants that is the difference
// between a handheld that keeps up and one that does not.

use crate::gfx::{Gfx, Offscreen, Rgba, Texture};
use anyhow::Result;
use std::ffi::CString;

/// How much smaller the blurred copy is than the screen.
///
/// A blur destroys detail by definition, so keeping it at full resolution is
/// paying for something nobody can see. A quarter in each direction is a
/// sixteenth of the pixels, and it makes the blur itself wider for free —
/// which is why the radius below is small.
const SHRINK: u32 = 4;

// Nine taps: the center and four either side, written out in the shader
// below. At a quarter scale that reaches sixteen screen pixels, which is where
// `blur(10px)` on a retina display lands.

// Locations are stated rather than left to the linker.
//
// `draw_full_quad` binds position to attribute 0 and texture coordinates to 1.
// Without a layout qualifier the linker assigns those numbers however it likes,
// and if it swaps them `gl_Position` receives the texture coordinates — which
// span 0..1 rather than -1..1, so the quad covers a single quadrant of the
// target and the other three keep whatever was in them. That is the backdrop
// "divided into four quarters", and it depends on the driver, which is why it
// showed on one machine and not in every measurement.
const VERTEX: &str = r#"
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec2 a_uv;
out vec2 v_uv;
void main() {
  v_uv = a_uv;
  gl_Position = vec4(a_pos, 0.0, 1.0);
}
"#;

/// One direction of the blur, plus the saturation the stylesheet asks for.
///
/// Weights are a five-tap Gaussian, normalized. Written out rather than
/// computed: it is nine samples in a loop that runs for every pixel of the
/// screen every frame, and a loop with a computed bound is the thing a mobile
/// shader compiler is worst at.
const FRAGMENT: &str = r#"
in vec2 v_uv;
out vec4 color;
uniform sampler2D u_source;
/// One texel, along whichever axis this pass runs. The other component is
/// zero, which is what makes one shader serve both passes.
uniform vec2 u_step;
uniform float u_saturate;

void main() {
  vec4 sum = texture(u_source, v_uv) * 0.2270270270;
  sum += texture(u_source, v_uv + u_step * 1.0) * 0.1945945946;
  sum += texture(u_source, v_uv - u_step * 1.0) * 0.1945945946;
  sum += texture(u_source, v_uv + u_step * 2.0) * 0.1216216216;
  sum += texture(u_source, v_uv - u_step * 2.0) * 0.1216216216;
  sum += texture(u_source, v_uv + u_step * 3.0) * 0.0540540541;
  sum += texture(u_source, v_uv - u_step * 3.0) * 0.0540540541;
  sum += texture(u_source, v_uv + u_step * 4.0) * 0.0162162162;
  sum += texture(u_source, v_uv - u_step * 4.0) * 0.0162162162;

  // saturate(150%), the other half of what the stylesheet asks for. Blurring
  // averages colors towards gray, and glass that has lost the color of what
  // is behind it reads as fog rather than as glass.
  float gray = dot(sum.rgb, vec3(0.2126, 0.7152, 0.0722));
  color = vec4(mix(vec3(gray), sum.rgb, u_saturate), 1.0);
}
"#;

pub struct Glass {
    program: u32,
    u_step: i32,
    u_saturate: i32,
    /// The backdrop, at a quarter size, and the scratch it is blurred through.
    /// Two, because a separated blur reads one and writes the other and cannot
    /// do both to the same texture.
    small: Offscreen,
    scratch: Offscreen,
    /// How far the blur reaches, as a multiple of one texel per tap.
    ///
    /// 0 makes every tap read the same texel, which is no blur at all — the
    /// "off" end of the setting rather than a special case.
    pub strength: f32,
    /// How saturated the result is. 1.0 leaves it alone.
    pub saturate: f32,
}

impl Glass {
    /// # Safety
    ///
    /// A context must be current.
    pub unsafe fn new(width: u32, height: u32) -> Result<Self> {
        unsafe {
            let version = crate::gfx::version_line()?;
            let precision = if version.contains(" es") {
                "precision highp float;\n"
            } else {
                ""
            };
            let program = crate::gfx::link(
                &format!("{version}\n{precision}{VERTEX}"),
                &format!("{version}\n{precision}{FRAGMENT}"),
            )?;
            let (w, h) = shrunk(width, height);
            Ok(Glass {
                u_step: uniform(program, "u_step"),
                u_saturate: uniform(program, "u_saturate"),
                program,
                small: Offscreen::new(w, h)?,
                scratch: Offscreen::new(w, h)?,
                strength: 1.5,
                saturate: 1.5,
            })
        }
    }

    /// Make the shrunken copies match a window this size, if they do not.
    ///
    /// # Safety
    ///
    /// A context must be current.
    pub unsafe fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        let (w, h) = shrunk(width, height);
        if self.small.size() == (w, h) {
            return Ok(());
        }
        unsafe {
            self.small = Offscreen::new(w, h)?;
            self.scratch = Offscreen::new(w, h)?;
        }
        Ok(())
    }

    /// What panels sample: the backdrop, blurred.
    pub fn blurred(&self) -> &Texture {
        &self.small.texture
    }

    /// How big that copy is. What fills it has to be drawn at this size, not
    /// at the window's.
    pub fn blurred_size(&self) -> (u32, u32) {
        self.small.size()
    }

    /// Draw the backdrop small, then blur it twice.
    ///
    /// `paint` is handed a renderer already pointed at the small texture, and
    /// should fill it — with the backdrop, in practice, which is the only
    /// thing behind the glass.
    ///
    /// # Safety
    ///
    /// A context must be current.
    pub unsafe fn capture(&mut self, gfx: &mut Gfx, paint: impl FnOnce(&mut Gfx)) {
        unsafe {
            gfx.draw_onto(&self.small, paint);
            // Across, into the scratch; then down, back into the small one. The
            // second pass reads what the first wrote, which is why there are
            // two textures and not one.
            let (w, h) = self.small.size();
            self.pass(
                gfx,
                &self.small.texture,
                &self.scratch,
                (1.0 / w as f32, 0.0),
            );
            self.pass(
                gfx,
                &self.scratch.texture,
                &self.small,
                (0.0, 1.0 / h as f32),
            );
        }
    }

    unsafe fn pass(&self, gfx: &mut Gfx, from: &Texture, into: &Offscreen, step: (f32, f32)) {
        unsafe {
            gfx.draw_onto(into, |_| {
                gl::UseProgram(self.program);
                gl::Uniform2f(self.u_step, step.0, step.1);
                gl::Uniform1f(self.u_saturate, self.saturate);
                gl::ActiveTexture(gl::TEXTURE0);
                gl::BindTexture(gl::TEXTURE_2D, from.raw());
                gl::Disable(gl::BLEND);
                crate::gfx::draw_full_quad();
            });
        }
    }

    /// Draw a panel: the blurred backdrop from behind it, under a tint.
    ///
    /// The tint is what makes it glass rather than a window — a pane has a
    /// color of its own, and the stylesheet's `--glass` is that color. Its
    /// alpha is how much of the blur shows through.
    #[allow(clippy::too_many_arguments)]
    pub fn panel(&self, gfx: &Gfx, screen: (f32, f32), x: f32, y: f32, w: f32, h: f32, tint: Rgba) {
        // Which part of the blurred copy sits behind this rectangle. The
        // texture is the whole screen shrunk, so the mapping is the panel's
        // own place on screen as a fraction of it — and it must be *this*
        // rectangle rather than the whole texture, or every panel shows the
        // same picture and the effect reads as wallpaper.
        //
        // Upside down in y, because the blurred copy was *rendered into* and a
        // texture rendered into holds its first row at the bottom — GL's
        // origin, not the layout's. Sampling it the right way up gives each
        // panel the mirror image of what is behind it, which a soft backdrop
        // hides completely and a backdrop with an edge in it does not: the
        // bars inside the pane sat a third of the screen from the bars behind
        // it.
        let source = (
            x / screen.0,
            1.0 - y / screen.1,
            (x + w) / screen.0,
            1.0 - (y + h) / screen.1,
        );
        gfx.image_part(self.blurred(), x, y, w, h, source, Rgba::WHITE);
        gfx.rect(x, y, w, h, tint);
    }
}

impl Drop for Glass {
    fn drop(&mut self) {
        unsafe { gl::DeleteProgram(self.program) }
    }
}

/// The size of the blurred copy. At least one pixel each way, because a window
/// dragged to nothing still has to draw.
fn shrunk(width: u32, height: u32) -> (u32, u32) {
    ((width / SHRINK).max(1), (height / SHRINK).max(1))
}

unsafe fn uniform(program: u32, name: &str) -> i32 {
    unsafe {
        let c = CString::new(name).unwrap();
        gl::GetUniformLocation(program, c.as_ptr())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nine taps that do not add to one is a blur that brightens or darkens
    /// the picture, which on a backdrop reads as the whole app changing
    /// exposure when a panel is drawn.
    #[test]
    fn the_blur_weights_sum_to_one() {
        let weights: Vec<f32> = FRAGMENT
            .lines()
            .filter_map(|line| {
                line.rsplit_once("* 0.")?
                    .1
                    .trim_end_matches(';')
                    .parse()
                    .ok()
            })
            .map(|n: f32| n / 10f32.powi(10))
            .collect();
        assert_eq!(
            weights.len(),
            9,
            "expected nine taps, found {}",
            weights.len()
        );
        let total: f32 = weights.iter().sum();
        assert!((total - 1.0).abs() < 0.001, "the taps sum to {total}");
    }

    #[test]
    fn the_small_copy_is_never_nothing() {
        assert_eq!(shrunk(1920, 1080), (480, 270));
        assert_eq!(
            shrunk(2, 2),
            (1, 1),
            "a window dragged to nothing still draws"
        );
        assert_eq!(shrunk(0, 0), (1, 1));
    }
}
