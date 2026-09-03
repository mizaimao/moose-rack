// The animated backdrop, as a shader.
//
// `ui/js/backdrop.js` is not CSS — it is WebGL2 with GLSL fragment shaders, a
// full-screen quad, and a handful of uniforms. On Mali-G52 the shading
// language maps to it one for one, which is why
// docs/archive/handheld-frontend.md says not to leave this until last: it is the most
// portable thing in the whole front end, and getting it up early is what
// proves the GL context works on the device at all.
//
// The shader is the same source, with `#version 300 es` swapped for the
// desktop's `#version 330 core` — GLES 3.0 and GL 3.3 differ in the version
// line and in whether precision qualifiers are required, and in nothing else
// that this uses.

use anyhow::Result;
use std::ffi::CString;

/// Shared by every style: the uniforms, the noise, and the vignette.
///
/// One program per style would mean a compilation each at startup and a copy
/// each of the color handling, so a style is a body spliced into this frame
/// and they cannot drift apart on how they read `u_strength` or how they
/// darken at the edges.
const HEAD: &str = r#"
uniform vec2  u_size;
uniform float u_time;
uniform vec3  u_low;
uniform vec3  u_mid;
uniform vec3  u_high;
uniform float u_strength;
uniform float u_speed;

float hash(vec2 p) {
  // Wrapped before the sine so the argument stays small however far the
  // coordinate has drifted. Time is unbounded here, and without the wrap the
  // sine is being asked for a value it cannot resolve after a few minutes of
  // running — which is what made the noisier styles band and march.
  p = fract(p * vec2(0.3183099, 0.3678794));
  p += dot(p, p + 19.19);
  return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

float noise(vec2 p) {
  vec2 i = floor(p), f = fract(p);
  f = f * f * (3.0 - 2.0 * f);
  return mix(mix(hash(i), hash(i + vec2(1, 0)), f.x),
             mix(hash(i + vec2(0, 1)), hash(i + vec2(1, 1)), f.x), f.y);
}

// Two octaves is enough for a backdrop and half the cost of four. This is
// drawn behind a library, not in a demo.
float fbm(vec2 p) {
  return noise(p) * 0.65 + noise(p * 2.1 + 4.7) * 0.35;
}

// Low -> mid -> high, so a scheme can be three colors rather than two.
vec3 ramp(float k) {
  k = clamp(k, 0.0, 1.0);
  return k < 0.5 ? mix(u_low, u_mid, k * 2.0) : mix(u_mid, u_high, (k - 0.5) * 2.0);
}

void main() {
  vec2 uv = gl_FragCoord.xy / u_size;
  vec2 aspect = vec2(u_size.x / u_size.y, 1.0);
  float t = u_time * u_speed;
  vec3 base;
"#;

const TAIL: &str = r#"
  // Darker towards the edges, and never fully bright in the middle either.
  // The grid sits on top of all of it, and text has to stay readable over the
  // brightest pixel this can produce rather than the average one.
  float d = distance(uv, vec2(0.5)) * 1.15;
  base *= 1.0 - smoothstep(0.15, 1.0, d) * 0.75;
  color = vec4(base * u_strength, 1.0);
}
"#;

const VERTEX: &str = r#"
in vec2 pos;
void main() { gl_Position = vec4(pos, 0.0, 1.0); }
"#;

/// One shape, and what the motion slider means for it.
///
/// `pace` exists because one slider across five shapes was a lie: every body
/// writes its own multipliers on `t`, and they are two decades apart. Blobs
/// drifts at 0.015 of it and Plasma sweeps at 0.31.
pub struct Style {
    pub id: &'static str,
    pub label: &'static str,
    pub pace: f32,
    pub body: &'static str,
}

pub const STYLES: &[Style] = &[
    Style {
        id: "blobs",
        label: "Blobs",
        pace: 1.7,
        body: r#"
      float a = noise(uv * aspect * 3.0 + vec2(t * 0.02, t * 0.013));
      float b = noise(uv * aspect * 6.0 - vec2(t * 0.011, t * 0.017));
      base = ramp(smoothstep(0.30, 0.85, a * 0.65 + b * 0.35));"#,
    },
    Style {
        id: "aurora",
        label: "Aurora",
        pace: 1.4,
        body: r#"
      float band = uv.y * 2.2 + fbm(vec2(uv.x * 2.0, t * 0.05)) * 1.6;
      float curtain = sin(band * 3.14159) * 0.5 + 0.5;
      curtain *= smoothstep(1.0, 0.15, uv.y);
      base = ramp(pow(curtain, 1.6));"#,
    },
    Style {
        id: "plasma",
        label: "Plasma",
        pace: 0.1,
        body: r#"
      vec2 p = (uv - 0.5) * aspect * 4.0;
      float v = sin(p.x + t * 0.35)
              + sin(p.y * 1.3 - t * 0.28)
              + sin((p.x + p.y) * 0.7 + t * 0.2)
              + sin(length(p) * 1.6 - t * 0.4);
      base = ramp(smoothstep(-1.2, 2.4, v));"#,
    },
    Style {
        id: "grid",
        label: "Grid",
        pace: 0.7,
        body: r#"
      vec2 q = (uv - vec2(0.5, 0.55)) * aspect;
      float depth = max(-q.y, 1e-3);
      vec2 g = vec2(q.x / depth, 1.0 / depth + t * 0.25);
      vec2 w = max(fwidth(g * 2.0), 1e-4);
      vec2 line = abs(fract(g * 2.0) - 0.5) / w;
      float mesh = 1.0 - min(min(line.x, line.y), 1.0);
      // Nothing above the horizon but a glow, and the mesh fades in below it:
      // the lines converge faster than the pixels can hold them right at the
      // vanishing line, and drawn hard that reads as noise rather than as
      // distance.
      float floorMask = step(0.0, -q.y) * smoothstep(0.0, 0.06, depth);
      float sky = 1.0 - smoothstep(0.0, 0.30, max(q.y, 0.0));
      base = ramp(mesh * 0.8 * floorMask + sky * 0.12);"#,
    },
    Style {
        id: "stars",
        label: "Drift",
        pace: 1.0,
        body: r#"
      vec2 p = uv * aspect * 18.0 + vec2(0.0, t * 0.08);
      vec2 cell = floor(p), f = fract(p);
      float glow = 0.0;
      for (int j = -1; j <= 1; j++) {
        for (int i = -1; i <= 1; i++) {
          vec2 o = vec2(float(i), float(j));
          vec2 c = cell + o;
          float star = hash(c);
          vec2 pos = f - o - vec2(hash(c + 3.1), hash(c + 7.7));
          float twinkle = 0.6 + 0.4 * sin(t * 1.6 + star * 40.0);
          glow += smoothstep(0.42, 0.0, length(pos)) * step(0.955, star) * twinkle;
        }
      }
      base = ramp(min(glow, 1.0));"#,
    },
    Style {
        id: "starfield",
        label: "Starfield",
        pace: 1.1,
        body: r#"
      base = u_low;
      for (int L = 0; L < 3; L++) {
        float f = float(L);
        float scale = 10.0 + f * 12.0;
        float speed = (f + 1.0) * 0.06;
        vec2 p = uv * aspect * scale + vec2(t * speed, 0.0);
        vec2 cell = floor(p), fr = fract(p);
        // The neighbours too. A star is scattered anywhere in its cell and its
        // glow has a radius, so one near an edge had the rest of it drawn by a
        // cell that was never asked — the same bug Drift had, and it looks the
        // same: points sliced flat along invisible straight lines.
        float glow = 0.0;
        for (int j = -1; j <= 1; j++) {
          for (int i = -1; i <= 1; i++) {
            vec2 o = vec2(float(i), float(j));
            vec2 c = cell + o;
            float star = hash(c + f * 17.0);
            vec2 off = fr - o - vec2(hash(c + 3.1), hash(c + 7.7));
            glow += smoothstep(0.30, 0.0, length(off)) * step(0.972, star);
          }
        }
        base = mix(base, u_high, min(glow, 1.0) * (0.35 + f * 0.32));
      }"#,
    },
    Style {
        id: "cubes",
        label: "Cubes",
        pace: 0.25,
        body: r#"
      // The PlayStation 2's opening scene: glass blocks turning slowly in a
      // blue haze. Towers is the scene *after* this one — the field of pillars,
      // one per save — and this is what plays before it.
      //
      // The blocks are real boxes, intersected rather than marched. A ray
      // against three slabs is a dozen operations and answers exactly: where it
      // went in, where it came out again, and which face it hit. A distance
      // field would be sixty steps a pixel for the same three numbers, which is
      // what ruled boxes out of Towers — here it is what makes seven of them
      // cost less than the haze behind them.
      vec2 q = (uv - 0.5) * aspect;

      // The haze, sampled in polar rather than in xy.
      //
      // Noise laid out in x and y gives puffs. The same noise laid out in angle
      // and log-radius stretches along rays out of the middle instead, and that
      // grain is most of what reads as light coming through smoke rather than
      // as cloud sitting in front of it.
      vec2 eye = q - vec2(-0.05 + sin(t * 0.05) * 0.04, 0.01 + cos(t * 0.043) * 0.03);
      float rad = max(length(eye), 0.02);
      // Along the ray direction, not along the angle.
      //
      // atan() has a cut at the negative x axis, and noise sampled on the angle
      // steps across it: what showed was a hard horizontal seam running left out
      // of the middle of the glow, through the brightest part of the picture.
      // normalize() carries the same information with no cut in it — a
      // continuous function of direction — so the grain still runs along the
      // rays and closes up all the way round.
      vec2 dir = eye / rad;

      // Sampled at a place another cloud decides.
      //
      // Two octaves of plain noise is an even wash. Looked at full size against
      // the original, the middle of the screen was one flat field with no shape
      // in it at all, where the original is wisps and lanes. Offsetting the
      // sample point by a second, coarser noise is what turns bands into wisps,
      // and it costs one more pair of lookups rather than the several more
      // octaves it would otherwise take to get the same detail.
      // Warped twice: folded sideways, and stretched outwards.
      //
      // A separate radial term was tried twice and is a mistake either way. A
      // value constant along a ray converges every streak on one pixel — coarse
      // it drew five fan-shaped brush strokes, fine it drew a lens flare — and
      // no weight worth having avoids both. Pushing the sample point along the
      // ray instead stretches the cloud's own features outwards, which is a
      // lobe rather than a streak, and it is the same two noise values already
      // fetched for the sideways fold.
      //
      // Two octaves of unwarped noise is an even wash: looked at full size
      // against the original, the middle of the screen was one flat field with
      // no shape in it where the original is wisps and lanes. This is what turns
      // bands into wisps, and it costs one pair of lookups rather than the
      // several more octaves it would take to get the same detail.
      vec2 warp = vec2(fbm(eye * 2.2 + vec2(t * 0.02, 0.0)),
                       fbm(eye * 2.2 + vec2(5.2, t * 0.017))) - 0.5;
      // The radial half fades out towards the middle, because dir flips sign
      // across the centre — the sample point jumped from one side of the cloud
      // to the other over a pixel, and what drew was a dark hole in the one
      // place on this screen that has to be the brightest.
      vec2 wq = eye + warp * 0.30 + dir * warp.y * 0.34 * smoothstep(0.0, 0.20, rad);

      // Three octaves, an octave and a bit apart. Two ran from scale 2.5 to 12
      // and the finest thing on screen was forty pixels across; the original has
      // structure in it down to a handful.
      float cloud = fbm(wq * 3.0 + vec2(t * 0.03, -t * 0.02)) * 0.50
                  + fbm(wq * 7.4 - vec2(t * 0.018, t * 0.026)) * 0.32
                  + fbm(wq * 17.0 + vec2(t * 0.012, t * 0.02)) * 0.18;

      // Stretched across the ramp rather than sitting near the top of it.
      //
      // This was 0.30 plus the cloud, which reached about 0.92 over the whole
      // middle of the screen — the top of the ramp everywhere, so one colour,
      // no lanes and no filaments however much structure the noise had in it.
      // The dark parts have to be dark for the bright parts to read as light.
      //
      // A gaussian, not a disc: the original has no edge to it anywhere, it
      // simply stops being there. Plus a much tighter one for the core, so the
      // haze has a source in it rather than being evenly lit throughout.
      float haze = exp(-rad * rad * 3.6) * smoothstep(0.14, 0.66, cloud) * 1.25
                 + exp(-rad * rad * 16.0) * 0.5;

      // The blocks. Nearest hit wins.
      //
      // Sorting seven translucent boxes per pixel is not affordable, and the
      // one in front is the one that should be seen. Additive would have been
      // order-free too — it is what Towers does — and it would have lost the
      // block crossing the haze as a silhouette, which is the shot.
      vec3 rd = normalize(vec3(q, 1.2));
      // How wide a pixel is, taken out here on purpose. A derivative inside the
      // loop below would be asked for inside a branch that not every pixel
      // takes, and a derivative under non-uniform control flow is undefined —
      // it is not slower there, it is wrong there.
      float qpx = max(fwidth(q.x), 1e-5);
      float front = 1e9;
      vec3 lit = vec3(0.0);
      float cover = 0.0;

      for (int i = 0; i < 7; i++) {
        float f = float(i);
        // Around a ring and drifting outwards, wrapping once it passes the
        // camera: the scene is a slow explosion that never finishes.
        float turn = fract(t * 0.017 + f / 7.0);
        float ring = f * 0.8975 + hash(vec2(f, 5.0)) * 0.45 + t * 0.02;
        float spread = 0.30 + turn * 1.25;
        vec3 cen = vec3(cos(ring) * spread * 1.15, sin(ring) * spread, mix(3.6, 1.1, turn));

        // Its own tumble, on two angles whose rates do not share a period, so
        // no two blocks ever fall into step.
        float ax = t * 0.09 + hash(vec2(f, 11.0)) * 6.3;
        float ay = t * 0.07 + hash(vec2(f, 13.0)) * 6.3;
        float sx = sin(ax), cx = cos(ax), sy = sin(ay), cy = cos(ay);
        // World to block, written out rather than multiplied: this is a Y turn
        // times an X turn, and forming it as two mat3 products is twenty-seven
        // multiplies per block per pixel to arrive at these four.
        //
        // Orthonormal, so the transpose is the inverse — which is why the
        // normal comes back out below with a multiply on the other side and no
        // second matrix.
        mat3 rot = mat3(cy, 0.0, -sy, sy * sx, cx, cy * sx, sy * cx, -sx, cy * cx);

        float h = 0.10 + hash(vec2(f, 17.0)) * 0.06;
        vec3 ro = rot * (-cen);
        vec3 rl = rot * rd;
        // Guarded rather than left to infinities. A ray exactly parallel to a
        // slab divides by zero and then subtracts one infinity from another,
        // which is a NaN, and a NaN inside min() takes the whole block with it.
        // sign() of zero is zero, so such a ray simply misses instead — a set
        // of rays with no area to it.
        vec3 inv = sign(rl) / max(abs(rl), 1e-6);
        vec3 mid = -inv * ro;
        vec3 ext = abs(inv) * h;
        vec3 t1 = mid - ext;
        vec3 t2 = mid + ext;
        float tn = max(max(t1.x, t1.y), t1.z);
        float tf = min(min(t2.x, t2.y), t2.z);

        if (tf > max(tn, 0.0) && tn < front) {
          front = tn;
          float t0 = max(tn, 0.0);
          // Where the ray went in, and where it came out. Both, because glass
          // is mostly the far side of itself.
          vec3 pin = ro + rl * t0;
          vec3 pout = ro + rl * tf;
          vec3 nl = -sign(rl) * step(t1.yzx, t1.xyz) * step(t1.zxy, t1.xyz);
          vec3 nor = nl * rot;

          float chord = tf - t0;
          float thru = clamp(chord / (h * 2.0), 0.0, 1.0);

          // Softened across one pixel at the silhouette.
          //
          // The ray grazes there, so the chord through the block goes to zero,
          // and the width of block that one pixel covers is that pixel's width
          // in world units at this depth. Drawn hard it is the one place a
          // half-resolution canvas shows, and it was also most of what this
          // style cost to animate: softening it took the blocks from 106 levels
          // a second to 32, which is calmer than Towers.
          float soft = smoothstep(0.0, qpx * cen.z / 1.2, chord);

          // Both sets of edges, and this is the difference between glass and
          // moulded plastic.
          //
          // Only the near face was drawn, so a block was an opaque shell with a
          // line round it — six flat panels, which is what a cheap plastic box
          // is. In the original you see the far edges *through* the near face:
          // a cube head-on shows a smaller square inside it, and a cube at an
          // angle shows the back corner crossing the front ones. That reading
          // through is the whole of what says there is a solid transparent
          // thing here rather than a painted hexagon.
          //
          // On whichever face was hit one of the three distances is zero — that
          // is the face — so the middle one is the distance to that face's
          // nearest edge, and it costs a median rather than a sort.
          vec3 ein = h - abs(pin);
          vec3 eout = h - abs(pout);
          float din = max(min(ein.x, ein.y), min(max(ein.x, ein.y), ein.z));
          float dout = max(min(eout.x, eout.y), min(max(eout.x, eout.y), eout.z));
          float lip = h * 0.26;
          float nearEdge = 1.0 - smoothstep(0.0, lip, din);
          // Dimmer, because it is being seen through the thickness of the
          // block. Equal weight and the near and far edges read as one flat
          // wireframe with no inside to it.
          float farEdge = (1.0 - smoothstep(0.0, lip, dout)) * 0.5;
          float edge = min(nearEdge + farEdge, 1.4);

          // Brighter where the surface turns away, which is what glass does and
          // paint does not. It is also most of the soft outline the original has
          // — a rim that comes from the geometry rather than from a drawn line.
          float fres = pow(1.0 - abs(dot(nor, rd)), 3.5);

          // How much light survives the crossing. Thin at the rim, dark through
          // the middle: smoked glass, not white perspex, and the reason a block
          // over the bright middle darkens it instead of covering it.
          float lets = exp(-thru * 1.4);

          // A little diffuse, signed so the three faces you can see are three
          // values rather than one. Small on purpose — carrying the block on
          // this term is exactly what made it look moulded.
          float sheen = (0.10 + 0.90 * (0.5 + 0.5 * dot(nor, vec3(-0.37, 0.55, -0.75)))) * 0.34;

          // Further away is dimmer, so the ring reads as depth rather than as a
          // circle of decals.
          float nearness = smoothstep(3.6, 1.1, cen.z);

          // The scheme's colour, with a neutral lift under it — the other way
          // round from how this started.
          //
          // It was grey plus a flat splash of the scheme, and flat is the word
          // that mattered: the scheme's share did not vary with the shading at
          // all, so every part of a block got the same amount of it and all the
          // contrast — the whole difference between an edge and a face — was
          // carried by the grey. That is why a lit edge came out at #7e8191,
          // which is not a colour, it is a shade.
          //
          // Now the scheme carries the shading and the grey only lifts it off
          // black. The edges go bright in the scheme's own hue rather than
          // towards white, and switching scheme changes the blocks instead of
          // tinting them: on Midnight an edge is #545c9b where it used to be
          // #7e8191, and on Moss it is a green rather than the same grey with a
          // suggestion of green in it.
          //
          // The neutral part is a third of what it was, which is the other half
          // of the same change: there is less grey to wash the hue out.
          float value = edge * 0.44 + fres * 0.30 + lets * 0.22 + sheen;
          // Down by 30 per cent, not 50: halving it put the blocks under the
          // haze almost everywhere. Both halves in the same proportion, so the
          // hue and the shading are what they were and only the level moved.
          lit = (vec3(0.02 + value * 0.40) * 0.30
               + u_high * (0.30 + value * 0.85)) * 0.7;
          // Two fifths opaque at the very most: still see-through, but present
          // enough to be a thing in the haze rather than a suggestion of one.
          //
          // Scaled down to a tenth rather than clipped at one, which is the
          // difference between keeping the shape and losing it — a clamp would
          // have flattened the edges and the rim into the faces and left a
          // uniform grey tile. Thin across a face and gathering at the edges and
          // the rim is still where a pane of glass actually stops light.
          cover = clamp(0.16 + edge * 0.46 + fres * 0.28 + lets * 0.14, 0.0, 1.0)
                * 0.40 * (0.32 + 0.68 * nearness) * soft;
        }
      }

      // The coloured sparks, and the one place in here that does not take its
      // colour from the scheme.
      //
      // In the original they are red, green and violet against the blue, and a
      // spark tinted to match the haze is not a spark — it is a bright patch of
      // haze. Five of them, two or three pixels across, so what is being
      // disobeyed is about fifteen pixels.
      vec3 sparks = vec3(0.0);
      for (int i = 0; i < 5; i++) {
        float f = float(i);
        float sa = t * 0.032 * (0.6 + hash(vec2(f, 23.0))) + hash(vec2(f, 29.0)) * 6.3;
        float sr = 0.14 + hash(vec2(f, 31.0)) * 0.40;
        vec2 sp = vec2(cos(sa) * 1.2, sin(sa * 1.3)) * sr;
        float sd = length(q - sp);
        // Evenly round the wheel, and nothing else. No two of them are ever
        // the same colour, and that is arithmetic rather than luck: five tones
        // at a fifth of the wheel apart are seventy-two degrees apart, always.
        //
        // Hashing the hue outright came out blue, blue, cyan, green, green —
        // five draws from one hat, with no red anywhere, which is the first
        // colour anybody remembers about this scene. Jittering an even spacing
        // by a hash fixed that and kept the failure in miniature: the gap
        // between two of them could close to twenty-nine degrees, which is two
        // greens. There is nothing for the jitter to buy here.
        float tone = f / 5.0;
        vec3 hue = 0.5 + 0.5 * cos(6.28318 * (tone + vec3(0.0, 0.33, 0.67)));
        // A core and a wide, faint halo. The core alone is a dead pixel at this
        // size; the halo is what makes it a light rather than a dot.
        //
        // Both the width and the drift rate are held back on purpose, and they
        // are still what this style costs. A bright four-pixel core crossing a
        // dark field is the largest step any pixel here takes, by a distance:
        // measured, the blocks and the haze together move 32 levels a second
        // and these five dots take that to 126 — the difference on the handheld
        // between redrawing five times a second and twenty-one. It is the same
        // reason Starfield scores as it does.
        sparks += hue * (exp(-sd * 155.0) + exp(-sd * 28.0) * 0.11);
      }

      base = ramp(clamp(haze, 0.0, 1.0));
      base = mix(base, lit, cover);
      base += sparks * 0.72;"#,
    },
    Style {
        id: "towers",
        label: "Towers",
        pace: 0.3,
        body: r#"
      // The rows cycle in depth: one reaching the front wraps to the back,
      // with the phases spread so they arrive evenly rather than in a pulse.
      // Additive, because the columns are light rather than surfaces — which
      // also means they never need sorting, and that is what lets a row wrap
      // past its neighbours without a seam.
      float horizon = 0.58;
      vec2 q = vec2((uv.x - 0.5) * aspect.x, uv.y);
      float ypx = max(fwidth(q.y), 1e-5);
      float glow = 0.0;

      for (int L = 0; L < 6; L++) {
        float turn = t * 0.06 + float(L) / 6.0;
        float phase = fract(turn);
        float z = mix(6.0, 0.55, phase);
        float s = 0.42 / z;

        // Where this row's floor meets the screen, and how much world reaches
        // across one screen unit at that depth.
        float yb = horizon - s * 0.9;
        float wx = q.x / s;

        // The row is slid sideways by its own hash, and re-seeded each time it
        // wraps: rows sharing an offset line up into corridors, and the same
        // rank of columns coming round again is a carousel.
        float row = float(L) * 13.0 + floor(turn);
        float slide = hash(vec2(row, 3.0)) * 7.0;
        float cx = floor(wx + slide);
        float fx = fract(wx + slide) - 0.5;

        // Most cells empty. A solid rank is a fence; the boot screen was a
        // scatter, because it was drawing what was on the memory card.
        float seed = hash(vec2(cx, row));
        float there = step(0.7, seed);
        float height = (0.35 + hash(vec2(cx, row + 1.7)) * 1.5) * s;
        float tall = max(height, 1e-4);

        // The bar, and a wider soft falloff around it. The halo is most of
        // what reads as translucent light rather than as a white rectangle.
        float wpx = max(fwidth(wx), 1e-4);
        float core = 1.0 - smoothstep(0.10 - wpx, 0.10 + wpx, abs(fx));
        float halo = exp(-abs(fx) * 9.0) * 0.35;

        // Up the column, in blocks. The seams flatten out once a block is
        // thinner than a pixel, which at the far end of the range they are —
        // drawn anyway they stop being blocks and become a shimmer.
        float up = q.y - yb;
        float k = clamp(up / tall, 0.0, 1.0);
        float bs = max(s * 0.22, 1e-4);
        float band = 0.3 * smoothstep(0.004, 0.022, bs);
        float box = (1.0 - band) + band * cos(up / bs * 6.28318);
        // Soft at the foot and at the cap rather than cut off. The canvas is
        // drawn at half the window's resolution and scaled up, and a hard step
        // along a vertical edge is the one place that shows.
        float column = smoothstep(-ypx, ypx, up)
                     * smoothstep(height + ypx, height - ypx, up);
        float lit = column * (mix(1.0, 0.4, k) * box + smoothstep(0.8, 1.0, k) * 0.9);

        // A little of the light escaping past the top of the stack.
        lit += step(height, up) * exp(-max(up - height, 0.0) * 26.0) * 0.35;

        // The reflection, mirrored in the plane and dimmer the further it
        // falls from the foot of the column.
        float below = yb - q.y;
        lit += step(0.0, below) * step(below, height) * exp(-below * 5.0) * 0.5;

        // Haze towards the back, and a fade at both ends of the depth range so
        // a row never pops in at either edge of it. Depth is a straight
        // function of the phase here, so the haze can be read off that rather
        // than off z.
        glow += lit * (core + halo) * there * (0.45 + 0.55 * phase)
              * smoothstep(0.0, 0.18, phase) * smoothstep(1.0, 0.82, phase);
      }

      // The plane itself: not black, and brightest where it meets the horizon,
      // which is what stops the lower half of the screen reading as a hole
      // rather than as a floor.
      //
      // Across the horizon, not off it. A step() here put the floor at 0.13 on one
      // side and 0 on the other, while the horizon glow below decays over about
      // a hundredth of the screen — so the two did not meet, and what was left
      // between them was a dark band sitting on a bright line. Measured on the
      // device: the horizon row is the brightest on screen and the four rows
      // directly above it are the darkest thing around them.
      //
      // The mask is one pixel wide, so the floor still ends where it ended; and
      // above it the same value falls away as haze rather than to nothing, which
      // is what makes the join continuous.
      float hpx = max(fwidth(q.y), 1e-5);
      float floorMask = smoothstep(horizon + hpx, horizon - hpx, q.y);
      float plane = floorMask * smoothstep(0.0, 0.9, q.y / horizon) * 0.13;
      plane += (1.0 - floorMask) * 0.13 * exp(-max(q.y - horizon, 0.0) * 7.0);
      plane += exp(-abs(q.y - horizon) * 90.0) * 0.1;
      base = ramp(min(glow * 0.8 + plane, 1.0));
      // The hottest cores go past the top of the ramp, towards white. Most of
      // these schemes have a dark navy for their high, and a light source that
      // never gets brighter than the wall behind it does not read as a light
      // source — it reads as a painted stripe. Only where several columns'
      // haloes overlap, so the field stays the color the scheme asked for.
      base += smoothstep(0.75, 1.6, glow) * 0.35;
"#,
    },
    Style {
        id: "tunnel",
        label: "Tunnel",
        pace: 0.35,
        body: r#"
      vec2 q = (uv - 0.5) * aspect;
      float r = max(length(q), 0.02);
      float a = atan(q.y, q.x);
      // 1/r is the depth: rings bunch towards the center because that is
      // where the tunnel is furthest away.
      float rings = sin(1.0 / r * 3.0 - t * 0.8) * 0.5 + 0.5;
      float spokes = sin(a * 8.0 + t * 0.15) * 0.5 + 0.5;
      base = ramp(rings * 0.7 * (0.55 + spokes * 0.45) * smoothstep(0.0, 0.35, r));"#,
    },
    Style {
        id: "ribbon",
        label: "Ribbon",
        pace: 0.4,
        body: r#"
      // The wave behind RetroArch's XMB, which it took from the PlayStation 3.
      //
      // Smooth bands of light crossing a gradient, and that is all it is. The
      // first version of this drew a wireframe grid in perspective running to a
      // horizon, which is a different picture entirely: theirs has no lines in
      // it anywhere, no mesh, and no depth. Reading the shader's name and
      // building what the word suggested cost a whole style.
      //
      // Which also makes it the cheapest thing on this list. There is no
      // surface to intersect — a ribbon is a line across the screen with a soft
      // falloff either side of it.
      vec2 p = vec2((uv.x - 0.5) * aspect.x, uv.y - 0.5);

      // The ground is a diagonal gradient, not a flat colour, and that alone is
      // half of what the picture is: theirs runs bright at one corner into deep
      // at the other, and the bands are only legible against it.
      float sweep = clamp(0.5 + p.x * 0.40 + p.y * 0.66, 0.0, 1.0);
      float lit = 0.16 + sweep * 0.36;

      for (int i = 0; i < 4; i++) {
        float f = float(i);
        float phase = f * 1.7 + hash(vec2(f, 3.0)) * 6.3;
        // Three sines along the band, so what travels is a fold in the ribbon
        // rather than the whole thing sliding up and down the screen.
        float mid = sin(p.x * 1.10 + t * 0.30 + phase) * 0.13
                  + sin(p.x * 2.05 - t * 0.21 + phase * 1.7) * 0.06
                  + sin(p.x * 0.55 + t * 0.13 + phase * 0.6) * 0.10;
        // Spread about the middle. Theirs are gathered across the centre of the
        // screen rather than spaced evenly down it.
        mid += (f - 1.5) * 0.05;

        // Broad. Theirs are sheets of light with most of the screen's width in
        // them, not streaks — narrow bands read as contrails, and four of them
        // read as a scribble.
        float halfw = 0.09 + hash(vec2(f, 9.0)) * 0.10;
        float band = 1.0 - smoothstep(0.0, halfw, abs(p.y - mid));
        // Squared for a soft core instead of a flat top, and added rather than
        // blended: where two ribbons cross, the overlap is brighter than either,
        // which is the whole of the effect.
        lit += band * band * (0.13 + hash(vec2(f, 11.0)) * 0.10);
      }

      base = ramp(clamp(lit, 0.0, 1.0));
"#,
    },
    Style {
        id: "sweep",
        label: "Sweep",
        pace: 0.9,
        body: r#"
      // Named band, not d: SHADER_TAIL declares its own float d for the
      // vignette, and a body declaring one too is a redeclaration — the shader
      // fails to compile, programFor returns null, and the style simply never
      // switches, with nothing on screen to say why.
      // (No backticks anywhere in here: this is inside a JS template literal.)
      float band = (uv.x * aspect.x + uv.y) * 0.7;
      base = ramp(0.5 + 0.5 * sin(band * 2.2 - t * 0.5));"#,
    },
    Style {
        id: "static",
        label: "Static",
        pace: 2.0,
        body: r#"
      // Three fields at incommensurable scales, each drifting on its own
      // irrational-ish vector, plus a slow wander of the sampling grid itself.
      //
      // Two fields on whole-number scales still shared a period: 3 and 7 line
      // up every 21 pixels and the eye finds that. These are 3.0, 5.7 and
      // 11.3, and the drift vectors are picked so no two are rational
      // multiples of each other — the combined pattern has no repeat short
      // enough to see. The last term moves the whole lattice, so even a static
      // pixel is sampling somewhere new each frame.
      vec2 wander = vec2(sin(t * 0.13), cos(t * 0.17)) * 40.0;
      float g1 = hash(floor((uv * u_size + wander) / 3.0)  + vec2(t * 37.7, t * 11.3));
      float g2 = hash(floor((uv * u_size + wander) / 5.7)  - vec2(t * 19.1, t * 43.9));
      float g3 = hash(floor((uv * u_size - wander) / 11.3) + vec2(t * 29.3, t * 7.1));
      base = ramp(g1 * 0.34 + g2 * 0.20 + g3 * 0.16 + 0.08);"#,
    },
];

/// What the motion slider sits at before anyone touches it.
///
/// From `ui/js/backdrop.js`, where it is shared across styles and each one's
/// `pace` scales it. The range there is 0 to 7.
pub const DEFAULT_SPEED: f32 = 4.0;

/// Just the ids and labels, for a settings list that must not depend on the
/// shader source being compiled.
pub const STYLE_LIST: &[(&str, &str)] = &[
    ("blobs", "Blobs"),
    ("aurora", "Aurora"),
    ("plasma", "Plasma"),
    ("grid", "Grid"),
    ("stars", "Drift"),
    ("starfield", "Starfield"),
    ("cubes", "Cubes"),
    ("towers", "Towers"),
    ("tunnel", "Tunnel"),
    ("ribbon", "Ribbon"),
    ("sweep", "Sweep"),
    ("static", "Static"),
];

/// The most any one pixel of each style moves in a second, at its own pace.
///
/// Measured, not estimated. `MOOSE_SDL_JITTER=1 cargo test -p moose-sdl --test
/// rendering` prints this table on a hidden context; `MOOSE_SDL_BENCH=motion`
/// prints the same thing from the running app, with a window on screen. It is
/// what decides how often each style has to be redrawn, and it has to be the
/// *worst* pixel rather than the average: a handful of stars crossing a dark
/// screen shift almost no average level and are the most obvious thing on it.
/// Starfield's average says it is the stillest style here and its worst says it
/// is nearly the jumpiest, and the second one is the one an eye agrees with.
pub const STYLE_JITTER: &[(&str, f32)] = &[
    ("aurora", 3.0),
    ("plasma", 6.0),
    ("blobs", 11.0),
    ("ribbon", 11.0),
    ("tunnel", 14.0),
    ("static", 24.0),
    ("sweep", 24.0),
    ("grid", 32.0),
    ("stars", 34.0),
    ("starfield", 39.0),
    ("towers", 62.0),
    ("cubes", 126.0),
];

/// The biggest step a pixel may take between two frames without the motion
/// reading as a series of jumps.
///
/// Six levels out of 255, against the dark grounds these are drawn on. Below
/// this an eye reports movement; above it, a slideshow.
const TOLERABLE_STEP: f32 = 6.0;

/// How often this style needs redrawing, in frames a second.
///
/// The whole point of the exercise: at one frame a second the app costs half a
/// percent of a core, and whether that *looks* like one frame a second depends
/// entirely on which backdrop is behind it. Aurora is the same picture either
/// way; Towers is a slideshow. So each gets the rate it actually needs instead
/// of every one of them paying for the fastest.
///
/// `speed` is the user's own multiplier: half speed is half the movement and
/// so half the frames.
pub fn needed_fps(style: &str, speed: f32) -> f64 {
    let jitter = STYLE_JITTER
        .iter()
        .find(|(id, _)| *id == style)
        .map(|(_, j)| *j)
        // A style nobody measured gets the fastest rate rather than the
        // slowest: being wrong here should cost battery, not look broken.
        .unwrap_or(62.0);
    ((jitter * speed.max(0.0) / TOLERABLE_STEP) as f64).clamp(1.0, 30.0)
}

/// A named color scheme, as the webview has them.
pub struct Named {
    pub id: &'static str,
    pub label: &'static str,
    pub scheme: Scheme,
    /// What the panels are tinted with. The webview picks this per scheme and
    /// the panels here were a fixed gray regardless — which is most of why they
    /// read as slabs rather than glass.
    pub glass: [f32; 3],
}

/// The schemes, ported from `ui/js/backdrop.js` so the two front ends offer
/// the same names and the same colors. `custom` is deliberately absent: a
/// color picker is miserable on a d-pad, and the handheld drops it.
///
/// The webview stores only two stops and mixes the middle, so the same
/// midpoint is computed here — the ramp then behaves identically.
pub const SCHEMES: &[Named] = &[
    Named {
        id: "midnight",
        label: "Midnight",
        scheme: Scheme {
            low: [0.0431, 0.051, 0.0863],
            mid: [0.1039, 0.1294, 0.2432],
            high: [0.1647, 0.2078, 0.4],
        },
        glass: [0.302, 0.5608, 0.8392],
    },
    Named {
        id: "frost",
        label: "Frost",
        scheme: Scheme {
            low: [0.0431, 0.0588, 0.0784],
            mid: [0.1216, 0.1862, 0.249],
            high: [0.2, 0.3137, 0.4196],
        },
        glass: [0.5608, 0.7216, 0.8471],
    },
    Named {
        id: "abyss",
        label: "Abyss",
        scheme: Scheme {
            low: [0.0235, 0.0353, 0.0471],
            mid: [0.047, 0.1451, 0.1745],
            high: [0.0706, 0.2549, 0.302],
        },
        glass: [0.2275, 0.6275, 0.7098],
    },
    Named {
        id: "moss",
        label: "Moss",
        scheme: Scheme {
            low: [0.0392, 0.0706, 0.0627],
            mid: [0.0804, 0.1804, 0.1392],
            high: [0.1216, 0.2902, 0.2157],
        },
        glass: [0.2471, 0.6196, 0.5255],
    },
    Named {
        id: "ember",
        label: "Ember",
        scheme: Scheme {
            low: [0.0784, 0.0431, 0.0353],
            mid: [0.2196, 0.0921, 0.0647],
            high: [0.3608, 0.1412, 0.0941],
        },
        glass: [0.7843, 0.5294, 0.2353],
    },
    Named {
        id: "rust",
        label: "Rust",
        scheme: Scheme {
            low: [0.0824, 0.0588, 0.0353],
            mid: [0.2255, 0.1431, 0.0628],
            high: [0.3686, 0.2275, 0.0902],
        },
        glass: [0.6902, 0.4157, 0.2078],
    },
    Named {
        id: "wine",
        label: "Wine",
        scheme: Scheme {
            low: [0.0745, 0.0392, 0.0549],
            mid: [0.202, 0.0824, 0.1412],
            high: [0.3294, 0.1255, 0.2275],
        },
        glass: [0.6902, 0.2902, 0.3333],
    },
    Named {
        id: "plum",
        label: "Plum",
        scheme: Scheme {
            low: [0.0706, 0.0392, 0.0863],
            mid: [0.1706, 0.1039, 0.2274],
            high: [0.2706, 0.1686, 0.3686],
        },
        glass: [0.4824, 0.3843, 0.7686],
    },
    Named {
        id: "slate",
        label: "Slate",
        scheme: Scheme {
            low: [0.0588, 0.0667, 0.0745],
            mid: [0.1294, 0.1471, 0.1666],
            high: [0.2, 0.2275, 0.2588],
        },
        glass: [0.4275, 0.4627, 0.5059],
    },
];

/// The panel tint for that scheme, or the first's.
pub fn glass_of(id: &str) -> [f32; 3] {
    SCHEMES.iter().find(|s| s.id == id).map(|s| s.glass).unwrap_or(SCHEMES[0].glass)
}

/// The scheme with that id, or the first.
pub fn scheme(id: &str) -> &'static Scheme {
    SCHEMES
        .iter()
        .find(|s| s.id == id)
        .map(|s| &s.scheme)
        .unwrap_or(&SCHEMES[0].scheme)
}

pub fn style(id: &str) -> &'static Style {
    STYLES.iter().find(|s| s.id == id).unwrap_or(&STYLES[0])
}

/// A color scheme, as the three stops the ramp blends between.
#[derive(Debug, Clone, Copy)]
pub struct Scheme {
    pub low: [f32; 3],
    pub mid: [f32; 3],
    pub high: [f32; 3],
}

impl Default for Scheme {
    /// The one the app opens on: a deep blue rising to a warm highlight.
    fn default() -> Self {
        Scheme {
            low: [0.04, 0.05, 0.10],
            mid: [0.10, 0.14, 0.30],
            high: [0.35, 0.30, 0.55],
        }
    }
}

pub struct Backdrop {
    program: u32,
    vao: u32,
    vbo: u32,
    uniforms: Uniforms,
    pub scheme: Scheme,
    pub strength: f32,
    pub speed: f32,
    pace: f32,
    label: &'static str,
}

struct Uniforms {
    size: i32,
    time: i32,
    low: i32,
    mid: i32,
    high: i32,
    strength: i32,
    speed: i32,
}

impl Backdrop {
    /// Compile it for the style named, against a context that is already
    /// current.
    ///
    /// # Safety
    ///
    /// Every call here is a GL call, and GL has no notion of which thread or
    /// which context it is being asked from — the caller has made one current
    /// and must not change it under us.
    pub unsafe fn build(video: &sdl2::VideoSubsystem, style_id: &str) -> Result<Self> {
        unsafe {
            gl::load_with(|name| video.gl_get_proc_address(name) as *const _);

            let chosen = style(style_id);
            // One dialect now: the context is ours, so it is the one we asked
            // for. This used to have to speak whatever SDL's renderer had
            // made, which on macOS was GLSL 1.20.
            let version = crate::gfx::version_line()?;
            let precision = if version.contains(" es") {
                "precision highp float;\n"
            } else {
                ""
            };
            let fragment = format!(
                "{version}\n{precision}out vec4 color;\n{HEAD}{}{TAIL}",
                chosen.body
            );
            let vertex = format!("{version}\n{VERTEX}");

            let program = crate::gfx::link(&vertex, &fragment)?;

            // One quad, as two triangles covering the clip volume. The vertex
            // shader passes it through: everything interesting happens per
            // fragment.
            let corners: [f32; 12] = [
                -1.0, -1.0, 1.0, -1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0, 1.0,
            ];
            // Vertex array objects are 3.0 and later. A legacy context binds
            // the buffer and describes it on every draw instead.
            let (mut vao, mut vbo) = (0, 0);
            gl::GenVertexArrays(1, &mut vao);
            gl::BindVertexArray(vao);
            gl::GenBuffers(1, &mut vbo);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                size_of_val(&corners) as isize,
                corners.as_ptr() as *const _,
                gl::STATIC_DRAW,
            );
            let pos = CString::new("pos").unwrap();
            let at = gl::GetAttribLocation(program, pos.as_ptr()).max(0) as u32;
            gl::EnableVertexAttribArray(at);
            gl::VertexAttribPointer(at, 2, gl::FLOAT, gl::FALSE, 0, std::ptr::null());
            gl::BindVertexArray(0);

            let uniform = |name: &str| {
                let c = CString::new(name).unwrap();
                gl::GetUniformLocation(program, c.as_ptr())
            };
            Ok(Backdrop {
                program,
                vao,
                vbo,
                uniforms: Uniforms {
                    size: uniform("u_size"),
                    time: uniform("u_time"),
                    low: uniform("u_low"),
                    mid: uniform("u_mid"),
                    high: uniform("u_high"),
                    strength: uniform("u_strength"),
                    speed: uniform("u_speed"),
                },
                scheme: Scheme::default(),
                strength: 0.5,
                // The webview's own default, and it matters: this is
                // multiplied by the style's `pace` and then by the small
                // constants in the body — Blobs drifts at 0.02 of it — so at
                // 1.0 the picture moves and nobody can tell.
                speed: DEFAULT_SPEED,
                pace: chosen.pace,
                label: chosen.label,
            })
        }
    }

    /// Draw one frame of it, filling whatever is bound.
    ///
    /// # Safety
    ///
    /// As `build`. The caller must also have flushed anything the SDL renderer
    /// had pending, or its batched state and ours interleave.
    pub unsafe fn draw(&self, width: f32, height: f32, seconds: f32) {
        unsafe {
            gl::UseProgram(self.program);
            gl::Uniform2f(self.uniforms.size, width, height);
            gl::Uniform1f(self.uniforms.time, seconds);
            gl::Uniform3fv(self.uniforms.low, 1, self.scheme.low.as_ptr());
            gl::Uniform3fv(self.uniforms.mid, 1, self.scheme.mid.as_ptr());
            gl::Uniform3fv(self.uniforms.high, 1, self.scheme.high.as_ptr());
            gl::Uniform1f(self.uniforms.strength, self.strength);
            gl::Uniform1f(self.uniforms.speed, self.speed * self.pace);
            gl::Disable(gl::BLEND);
            gl::Disable(gl::DEPTH_TEST);
            gl::BindVertexArray(self.vao);
            gl::DrawArrays(gl::TRIANGLES, 0, 6);
            gl::BindVertexArray(0);
            gl::UseProgram(0);
        }
    }
}

impl Backdrop {
    pub fn style_label(&self) -> &'static str {
        self.label
    }
}

impl Drop for Backdrop {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteProgram(self.program);
            gl::DeleteBuffers(1, &self.vbo);
            gl::DeleteVertexArrays(1, &self.vao);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every style has to splice into the shared frame, and the frame declares
    /// the things they all use. A body naming something the head does not
    /// define is a shader that fails to compile on the device and nowhere
    /// else, because nothing here compiles it without a context.
    #[test]
    fn every_style_only_uses_what_the_frame_provides() {
        for style in STYLES {
            for name in ["uv", "aspect", "t", "base"] {
                assert!(
                    HEAD.contains(name) || style.body.contains(name),
                    "{}: {name} is used by nobody",
                    style.id
                );
            }
            assert!(
                style.body.contains("base ="),
                "{} never fills base",
                style.id
            );
            assert!(style.pace > 0.0, "{} has no pace", style.id);
        }
    }

    /// The motion slider means the same thing to every shape, which is what
    /// `pace` is for — and the numbers are two decades apart, so a typo is
    /// invisible.
    #[test]
    fn the_paces_span_the_range_they_are_meant_to() {
        let fastest = STYLES.iter().map(|s| s.pace).fold(f32::MIN, f32::max);
        let slowest = STYLES.iter().map(|s| s.pace).fold(f32::MAX, f32::min);
        assert!(
            fastest / slowest > 10.0,
            "the paces are all the same, so the slider is a lie"
        );
    }

    /// Where the fragment color goes is the preamble's business and nobody
    /// else's. It was declared in both — the shared frame *and* the modern
    /// preamble — which is a duplicate on one dialect and a compile error on
    /// the other, and the error names a line the body does not contain.
    /// Where the fragment color goes is `build`'s business and nobody
    /// else's. It was declared in both once — the shared frame and the
    /// preamble — which is a duplicate on one dialect and a compile error on
    /// the other, naming a line the body does not contain.
    #[test]
    fn the_shared_frame_does_not_declare_the_output() {
        assert!(
            !HEAD.contains("out vec4"),
            "the frame declares the output as well"
        );
    }

    #[test]
    fn an_unknown_style_falls_back_rather_than_failing() {
        assert_eq!(style("nonsense").id, STYLES[0].id);
        assert_eq!(style("aurora").id, "aurora");
    }
}

#[cfg(test)]
mod pacing {
    use super::*;

    /// Every style in the list has a measured jitter, or the fallback silently
    /// gives it the fastest rate and nobody notices the table went stale.
    #[test]
    fn every_style_has_been_measured() {
        for (id, label) in STYLE_LIST {
            assert!(
                STYLE_JITTER.iter().any(|(m, _)| m == id),
                "{label} ({id}) is not in STYLE_JITTER; run it with MOOSE_SDL_JITTER=1"
            );
        }
    }

    /// The still ones cost one frame a second and the jumpy ones cost more.
    #[test]
    fn a_still_backdrop_is_not_redrawn_thirty_times_a_second() {
        assert_eq!(needed_fps("aurora", 1.0), 1.0);
        assert_eq!(needed_fps("plasma", 1.0), 1.0);
        assert!(needed_fps("towers", 1.0) > needed_fps("blobs", 1.0));
        // Half the movement is half the frames.
        assert!(needed_fps("towers", 0.5) < needed_fps("towers", 1.0));
        // Stopped is still one, not zero: the clock and the rest still need a
        // frame now and then, and the caller decides whether to animate at all.
        assert_eq!(needed_fps("towers", 0.0), 1.0);
        // Nothing exceeds the cap, whatever the speed is set to.
        assert_eq!(needed_fps("towers", 10.0), 30.0);
    }
}
