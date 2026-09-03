// An animated shader behind the library.
//
// WebGL2, which every webview this app runs in has had for years — WKWebView on
// macOS, Chromium's WebView2 on Windows, WebKitGTK on Linux. Not WebGPU, which
// is the newer API and still patchy across those three.
//
// It is a single full-screen quad running one fragment shader. There is no
// geometry, no textures and no library: the whole scene is arithmetic on the
// pixel coordinate, which is why it costs almost nothing and needs no
// dependency.
//
// Deliberately quiet. This sits behind cover art that the eye is supposed to be
// on, so it moves slowly and stays dark. A backdrop that competes with the
// artwork is worse than no backdrop.

/// Colors are read from the stylesheet rather than hardcoded, so the shader
/// follows whatever theme is in force instead of being a second place colors
/// have to be kept in sync.
function themeColor(name, fallback) {
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const m = raw.match(/^#?([0-9a-f]{6})$/i);
  if (!m) return fallback;
  const n = parseInt(m[1], 16);
  return [(n >> 16) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
}

const VERTEX = `#version 300 es
in vec2 pos;
void main() { gl_Position = vec4(pos, 0.0, 1.0); }`;

// Two layers of drifting value-noise, tinted between the theme's own two
// colors, plus a vignette so the edges fall away and the grid stays readable.
/// What every style shares: the uniforms, the noise, and the vignette.
///
/// One program per style would mean five compilations at startup and five
/// times the code that reads a color scheme. Instead each style is a body
/// that fills `base`, spliced into this frame — so they cannot drift apart on
/// how they read `u_strength`, how they darken at the edges, or how they take
/// the two scheme colors.
const SHADER_HEAD = `#version 300 es
precision highp float;

// highp, not mediump, and this is the whole reason Static and Rain looked like
// they repeated every few frames.
//
// hash() below is sin(dot(p, k)) * 43758.5453 taken fract. Static feeds it
// coordinates in the hundreds plus a time term that grows without bound, and
// mediump carries about three decimal digits — so sin of a large argument
// loses every bit that mattered, fract of the result lands on a handful of
// values, and the "noise" becomes a visible band pattern marching across the
// screen. It was never the shape of the effect; it was the arithmetic.
out vec4 color;

uniform vec2  u_size;
uniform float u_time;
uniform vec3  u_low;
uniform vec3  u_mid;
uniform vec3  u_high;
uniform float u_strength;
uniform float u_speed;

float hash(vec2 p) {
  // Wrapped before the sine so the argument stays small however far the
  // coordinate has drifted. Time is unbounded here — Static adds t * 37 — and
  // without the wrap the sine is being asked for a value it cannot resolve
  // even at highp after a few minutes of running.
  p = fract(p * vec2(0.3183099, 0.3678794));
  p += dot(p, p + 19.19);
  return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

// Value noise: hash the four lattice corners and smoothly blend between them.
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
//
// Every style already blends between two colors; this is the same call with
// a stop in the middle, so a two-color scheme sets the middle halfway and
// nothing changes for it. No backticks in here: this whole block is inside a
// JS template literal, and one would end the string.
vec3 ramp(float k) {
  k = clamp(k, 0.0, 1.0);
  return k < 0.5 ? mix(u_low, u_mid, k * 2.0) : mix(u_mid, u_high, (k - 0.5) * 2.0);
}

void main() {
  vec2 uv = gl_FragCoord.xy / u_size;
  vec2 aspect = vec2(u_size.x / u_size.y, 1.0);
  float t = u_time * u_speed;
  vec3 base;
`;

const SHADER_TAIL = `
  // Darker towards the edges, and never fully bright in the middle either. The
  // grid sits on top of all of it and text has to stay readable over the
  // brightest pixel this can produce, not the average one.
  float d = distance(uv, vec2(0.5)) * 1.15;
  base *= 1.0 - smoothstep(0.15, 1.0, d) * 0.75;

  color = vec4(base * u_strength, 1.0);
}`;

/// The styles, as the body each one contributes.
///
/// There was one. "Blobs" is a fair description of it and a poor choice of
/// only option: the whole point of a backdrop is that it suits the room, and
/// one shape suits one room. Each of these takes the same two scheme colors
/// so switching style does not also change the palette.
///
/// `pace` is what the Motion slider means for this style, and it exists
/// because the slider was a lie shared between five shapes. Every body writes
/// its own multipliers on `t`, and they are two decades apart: Blobs drifts at
/// 0.015 of it and Plasma sweeps at 0.31, twenty times faster for the same
/// number on the slider. Setting it where Plasma was watchable left Blobs and
/// Aurora looking like still images. The slider now says how fast *this*
/// backdrop should go, and each style carries the factor that makes that true
/// — so the setting survives switching style, which is the whole point of one
/// slider rather than five.
export const BACKDROPS = [
  {
    id: "blobs",
    label: "Blobs",
    hint: "Soft clouds drifting at two scales. The original, and the quietest.",
    pace: 1.7,
    defaults: { strength: 0.5 },
    body: `
      float a = noise(uv * aspect * 3.0 + vec2(t * 0.02, t * 0.013));
      float b = noise(uv * aspect * 6.0 - vec2(t * 0.011, t * 0.017));
      base = ramp(smoothstep(0.30, 0.85, a * 0.65 + b * 0.35));`,
  },
  {
    id: "aurora",
    label: "Aurora",
    hint: "Slow vertical curtains, brighter where they fold over each other.",
    pace: 1.4,
    defaults: { strength: 0.6 },
    body: `
      float band = uv.y * 2.2 + fbm(vec2(uv.x * 2.0, t * 0.05)) * 1.6;
      float curtain = sin(band * 3.14159) * 0.5 + 0.5;
      curtain *= smoothstep(1.0, 0.15, uv.y);
      base = ramp(pow(curtain, 1.6));`,
  },
  {
    id: "plasma",
    label: "Plasma",
    hint: "Interfering sine waves. The oldest trick on this list, and the one "
      + "that looks most like a demo from 1993.",
    // The one that made the slider a lie: four sines at 0.2–0.4 of `t`, where
    // Blobs drifts at 0.015 of it.
    pace: 0.1,
    defaults: { strength: 0.8 },
    body: `
      vec2 p = (uv - 0.5) * aspect * 4.0;
      float v = sin(p.x + t * 0.35)
              + sin(p.y * 1.3 - t * 0.28)
              + sin((p.x + p.y) * 0.7 + t * 0.2)
              + sin(length(p) * 1.6 - t * 0.4);
      base = ramp(smoothstep(-1.2, 2.4, v));`,
  },
  {
    id: "grid",
    label: "Grid",
    hint: "A horizon and a grid running to it. Steadier than the rest: the "
      + "lines do not move, only the light on them.",
    // `gl_FragCoord.y` counts from the bottom, so the floor is where `uv.y` is
    // *small* — and the old horizon at 0.18 put it below the bottom of the
    // screen. Everything actually on show was the clamp: `max(p.y, 0.04)`
    // froze the perspective divide across the whole lower fifth, so the lines
    // stopped converging at a fixed height and ran straight down from it, and
    // `fwidth` of a frozen coordinate is zero — a divide by zero smeared along
    // that same seam. Horizon in the upper half, depth measured downwards from
    // it, and the divide guarded rather than the input clamped.
    pace: 0.7,
    defaults: { strength: 1.0 },
    body: `
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
      base = ramp(mesh * 0.8 * floorMask + sky * 0.12);`,
  },
  {
    id: "stars",
    label: "Drift",
    hint: "Points of light, sparse and slow. Almost nothing on screen, which "
      + "on an OLED is almost nothing lit.",
    // A star is scattered inside a cell of an invisible grid, and its glow has
    // a radius of 0.42 of a cell — so any star sitting nearer than that to a
    // cell edge had the rest of its glow drawn by the neighbouring cell, which
    // was not asking about it. What showed was a field of points sliced flat
    // along straight lines, always the same lines, because the grid does not
    // move with the drift. Every point has to ask its eight neighbours too.
    pace: 1,
    defaults: { speed: 3.0, strength: 1.0 },
    body: `
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
      base = ramp(min(glow, 1.0));`,
  },
  {
    id: "starfield",
    label: "Starfield",
    hint: "Three layers of stars at different speeds. The attract-mode look, "
      + "and the one that reads as depth without moving much.",
    pace: 1.1,
    defaults: { speed: 3.0, strength: 1.0 },
    body: `
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
      }`,
  },
  {
    id: "cubes",
    label: "Cubes",
    hint: "Glass blocks turning in a blue haze. The PlayStation 2's opening "
      + "scene — Towers is the one that comes after it.",
    // Slower than Towers. This scene has no cycle in it to hurry: the blocks
    // drift out and wrap round, and at anything brisker it stops being a room
    // you are floating in and becomes something being thrown at you.
    pace: 0.25,
    defaults: { strength: 0.75 },
    body: `
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
      base += sparks * 0.72;`,
  },
  {
    id: "towers",
    label: "Towers",
    hint: "Columns of light standing on a dark plane that reflects them. The "
      + "second half of the PlayStation 2 boot, which drew one for every save "
      + "you had.",
    // Six rows of billboards, not a ray march. A distance field of boxes is
    // sixty-odd steps per pixel, and this is drawn behind cover art on a
    // machine that may also be running an emulator — the perspective divide is
    // the only 3D in it, and at a glance nothing here is missing.
    pace: 0.3,
    defaults: { strength: 0.7 },
    body: `
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
      base += smoothstep(0.75, 1.6, glow) * 0.35;`,
  },
  {
    id: "tunnel",
    label: "Tunnel",
    hint: "Rings running away from the middle. The oldest perspective trick "
      + "there is, and still the most hypnotic.",
    pace: 0.35,
    defaults: { strength: 0.7 },
    body: `
      vec2 q = (uv - 0.5) * aspect;
      float r = max(length(q), 0.02);
      float a = atan(q.y, q.x);
      // 1/r is the depth: rings bunch towards the center because that is
      // where the tunnel is furthest away.
      float rings = sin(1.0 / r * 3.0 - t * 0.8) * 0.5 + 0.5;
      float spokes = sin(a * 8.0 + t * 0.15) * 0.5 + 0.5;
      base = ramp(rings * 0.7 * (0.55 + spokes * 0.45) * smoothstep(0.0, 0.35, r));`,
  },
  {
    id: "ribbon",
    label: "Ribbon",
    hint: "A wave folding under a wireframe sheet. The PlayStation 3's menu "
      + "background, which is what RetroArch draws behind its own.",
    pace: 0.4,
    // Brighter than the rest by default. Every other style here has hot spots —
    // a star, a column, a spark — and has to be held down so text stays
    // readable over the brightest pixel it can make. This one is a gradient
    // with soft bands on it and its brightest pixel is its average, so the same
    // number that suits Starfield leaves it looking switched off.
    defaults: { strength: 0.68 },
    body: `
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
`,
  },
  {
    id: "sweep",
    label: "Sweep",
    hint: "A slow diagonal wash. The cheapest thing here — two sines and no "
      + "noise at all — and the quietest behind artwork.",
    pace: 0.9,
    defaults: { speed: 2.0, strength: 0.7 },
    body: `
      // Named band, not d: SHADER_TAIL declares its own float d for the
      // vignette, and a body declaring one too is a redeclaration — the shader
      // fails to compile, programFor returns null, and the style simply never
      // switches, with nothing on screen to say why.
      // (No backticks anywhere in here: this is inside a JS template literal.)
      float band = (uv.x * aspect.x + uv.y) * 0.7;
      base = ramp(0.5 + 0.5 * sin(band * 2.2 - t * 0.5));`,
  },
  {
    id: "static",
    label: "Static",
    hint: "Untuned television. One hash per pixel, so it costs almost nothing "
      + "— and on an OLED it is the busiest thing on this list.",
    pace: 2.0,
    defaults: { speed: 6.0, strength: 1.0 },
    // No motion control. The field is re-hashed from scratch every frame at
    // any speed, so the slider changes the numbers and nothing you can see —
    // and a control that does nothing is worse than no control. The default
    // is the cheapest rate that still looks like static.
    motion: false,
    body: `
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
      base = ramp(g1 * 0.34 + g2 * 0.20 + g3 * 0.16 + 0.08);`,
  },
];

export function backdropStyle(id) {
  return BACKDROPS.find((b) => b.id === id) ?? BACKDROPS[0];
}

/// The whole fragment shader for one style: the shared frame with that style's
/// body spliced into it.
///
/// Exported for `tools/backdrop-preview.html`, which compiles exactly what the
/// app compiles rather than a copy of it — a preview that reassembled the
/// pieces itself would be a second place the frame lives, and the first thing
/// it would fail to notice is a change to the frame.
export function fragmentFor(id) {
  return SHADER_HEAD + backdropStyle(id).body + SHADER_TAIL;
}

function compile(gl, type, src) {
  const sh = gl.createShader(type);
  gl.shaderSource(sh, src);
  gl.compileShader(sh);
  if (!gl.getShaderParameter(sh, gl.COMPILE_STATUS)) {
    console.warn("backdrop shader failed:", gl.getShaderInfoLog(sh));
    gl.deleteShader(sh);
    return null;
  }
  return sh;
}

let running = null;

/// What the backdrop looks like, as the user set it.
///
/// Kept in localStorage rather than config.toml: it is a per-screen preference,
/// and the machine driving a television wants different values from the laptop.
// Strength well below 1. At full it is a bright glow across the middle of the
// screen that the cover art has to compete with — the backdrop's whole job is
// to sit behind the artwork, not next to it.
const DEFAULTS = {
  speed: 4,
  strength: 0.32,
  low: "",
  high: "",
  preset: "midnight",
  style: "blobs",
};

/// Dark palettes, because this sits behind cover art at night on a television.
///
/// Each is a dark base and a slightly-lit accent; the shader blends between
/// them, so the pair is the whole scheme. Kept deliberately low in value — a
/// bright pair produces a backdrop that competes with the artwork no matter
/// what the brightness slider says.
/// The gradient a scheme resolves to, or the user's own on "custom".
///
/// An unknown id falls through to the stored colors as well: schemes can be
/// renamed or dropped, and a settings file naming one that no longer exists
/// should leave the window looking like something rather than nothing.
export function presetColors(cfg) {
  // Every scheme, not just the two-color pairs. This searched `SCHEMES`, so a
  // single color or a spectrum found nothing, returned the config's own empty
  // strings, and fell back to the theme — which is why all eighteen of them
  // rendered identically and looked like the mono scheme.
  const p = ALL_SCHEMES.filter(Boolean).find((x) => x.id === cfg.preset);
  if (!p || p.id === "custom") return { low: cfg.low, mid: cfg.mid, high: cfg.high };
  return { low: p.low, mid: p.mid, high: p.high };
}

/// Motion, as the slider expresses it.
///
/// Below about 3 the drift is too slow to read as movement at all — it looks
/// like a still image with a rendering cost. The slider covers the range that
/// actually differs.
export const SPEED_MIN = 0;
export const SPEED_MAX = 7;

/// Settings for one style, layered over the shared ones.
///
/// Each shape wants different numbers and the single set could not hold them:
/// Scanlines at the brightness that suits Drift is a white screen, and Tunnel
/// at Blobs' speed is a migraine. Stored under the style's own id and layered
/// over the shared values, so anything never touched for a style still follows
/// the shared setting rather than freezing at whatever it was the first time
/// that style was opened.
function perStyle() {
  try {
    return JSON.parse(localStorage.getItem("backdropPerStyle") || "{}");
  } catch {
    return {};
  }
}

/// Store `patch` against `style`, keeping the rest of that style's overrides.
export function saveStyleSettings(style, patch) {
  const all = perStyle();
  all[style] = { ...(all[style] || {}), ...patch };
  localStorage.setItem("backdropPerStyle", JSON.stringify(all));
  const merged = backdropSettings();
  if (live) live(merged);
  window.__TAURI__?.event?.emit?.("backdrop-settings", merged);
  return merged;
}

/// What this style overrides, if anything. Empty object when it follows the
/// shared settings for everything.
export function styleSettings(style) {
  return perStyle()[style] || {};
}

/// What a style starts at, before anyone touches it.
///
/// Some shapes are wrong at the shared numbers and always will be: Scanlines
/// at 32% brightness is a lit gray screen, Static at 32% is a snowstorm over
/// the artwork. The style carries its own answer and the shared settings are
/// the fallback for everything that has no opinion.
export function styleDefaults(style) {
  const own = BACKDROPS.find((b) => b.id === style)?.defaults || {};
  let shared;
  try {
    shared = { ...DEFAULTS, ...JSON.parse(localStorage.getItem("backdropSettings") || "{}") };
  } catch {
    shared = { ...DEFAULTS };
  }
  return { speed: shared.speed, strength: shared.strength, ...own };
}

/// Forget one style's overrides, putting it back on its own defaults.
export function clearStyleSettings(style) {
  const all = perStyle();
  // Back to the style's own defaults rather than to nothing: dropping the
  // overrides entirely would leave Scanlines on the shared brightness, which
  // is the setting it exists to disagree with.
  const own = BACKDROPS.find((b) => b.id === style)?.defaults;
  if (own) all[style] = { ...own };
  else delete all[style];
  localStorage.setItem("backdropPerStyle", JSON.stringify(all));
  const merged = backdropSettings();
  if (live) live(merged);
  window.__TAURI__?.event?.emit?.("backdrop-settings", merged);
  return merged;
}

export function backdropSettings() {
  let stored;
  try {
    stored = { ...DEFAULTS, ...JSON.parse(localStorage.getItem("backdropSettings") || "{}") };
  } catch {
    stored = { ...DEFAULTS };
  }
  // The style's own defaults sit under its overrides, and both over the
  // shared settings.
  const own = BACKDROPS.find((b) => b.id === stored.style)?.defaults || {};
  stored = { ...stored, ...own, ...(perStyle()[stored.style] || {}) };
  // Only a ceiling now. The floor was 3 — 300% — which is why the motion
  // slider refused to go below that however far it was dragged, and why a
  // style whose default is slower could not keep it.
  const raw = Number(stored.speed);
  stored.speed = Number.isFinite(raw) ? Math.max(0, raw) : DEFAULTS.speed;
  return stored;
}

/// Apply settings to the running shader. Does not store and does not announce.
///
/// The receiving end of the event, and separate from `saveBackdropSettings` for
/// exactly that reason: the listener used to call the saving version, which
/// emitted again, which the emitting window also received. Every drag of a
/// color slider fed itself back round and the backdrop flickered.
export function applyBackdropSettings(cfg) {
  if (live) live({ ...backdropSettings(), ...(cfg || {}) });
}

export function saveBackdropSettings(next) {
  // Built from the *shared* settings, never from `backdropSettings()`: that
  // one layers the current style's overrides on top, so saving anything would
  // copy those overrides into the shared values and every later style would
  // inherit them. Switching from Tunnel to Rain carried Tunnel's brightness
  // across and then kept it for good.
  let shared;
  try {
    shared = { ...DEFAULTS, ...JSON.parse(localStorage.getItem("backdropSettings") || "{}") };
  } catch {
    shared = { ...DEFAULTS };
  }
  const merged = { ...shared, ...next };
  localStorage.setItem("backdropSettings", JSON.stringify(merged));
  // What the shader is given still includes the chosen style's overrides.
  const effective = backdropSettings();
  // Applied live rather than on restart: a color picker you cannot see the
  // result of is a color picker nobody can use.
  if (live) live(effective);
  // ...and told to the other window, because the controls live in Settings and
  // the shader lives in the library. Calling startBackdrop from the settings
  // window put the canvas in *that* document, so changing the app's background
  // changed the background of the settings panel and nothing else.
  window.__TAURI__?.event?.emit?.("backdrop-settings", effective);
  return effective;
}

/// Whether the library window is showing a backdrop.
///
/// Read from storage rather than from `running`, so the settings window — which
/// never renders one — reports the state of the window that does. localStorage
/// is shared between them; they are the same origin.
export function backdropWanted() {
  return localStorage.getItem("backdrop") === "on";
}

/// Turn it on or off in whichever window renders the library.
export function setBackdropWanted(on) {
  localStorage.setItem("backdrop", on ? "on" : "off");
  window.__TAURI__?.event?.emit?.("backdrop-toggle", on);
  // In the library window itself, act immediately rather than waiting for the
  // event to come back around.
  if (document.getElementById("list")) {
    if (on) startBackdrop();
    else stopBackdrop();
  }
  return on;
}

/// Set while running, so settings changes reach the shader without restarting
/// it — recreating the GL context on every slider tick would stutter.
let live = null;

function rgb(hex, fallbackVar, fallbackRgb) {
  const m = String(hex || "").match(/^#?([0-9a-f]{6})$/i);
  if (m) {
    const n = parseInt(m[1], 16);
    return [(n >> 16) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
  }
  return themeColor(fallbackVar, fallbackRgb);
}

/// Start the backdrop. Returns a stop function, or null when it could not run —
/// an old driver, a software renderer, a webview with WebGL switched off. The
/// app is fully usable without it, so every failure here is silent.
export function startBackdrop() {
  // A measuring switch, never set in normal use. The shader runs on a canvas
  // the size of the window and never stops, so it is one of the few things in
  // the app that could plausibly cost tens of megabytes on its own — and the
  // only way to know is to be able to turn it off.
  if (globalThis.__MOOSE_FLAGS?.includes("no-backdrop")) return;
  try {
    return build();
  } catch (e) {
    // The module says every failure in here is silent, and for one version it
    // was not: a reference error thrown out of this function escaped into the
    // init that called it and took the tab row, the settings button and the
    // page background with it. Decoration cannot be allowed to do that,
    // whatever mistake is in it.
    console.warn("backdrop failed to start:", e);
    return null;
  }
}

function build() {
  if (running) return running;

  const canvas = document.createElement("canvas");
  canvas.id = "backdrop";
  const gl = canvas.getContext("webgl2", {
    antialias: false,
    // The backdrop is drawn once per frame and never read back; telling the
    // driver so lets it skip preserving the buffer between frames.
    preserveDrawingBuffer: false,
    powerPreference: "low-power",
    // Opaque on Android, transparent everywhere else.
    //
    // This device's WebView rasterises the page's own background lighter than
    // it is asked to — rgb(20, 22, 26) comes out #313236, the same colour with
    // twelve per cent of white in it. Not compositing: SurfaceFlinger reports
    // the layer as alpha=1, blend=NONE, identity colour transform, so the wash
    // is in the raster. Canvas pixels are exempt from whatever does it, which
    // is the one lever there is.
    //
    // With `alpha: false` the drawing buffer is opaque, so the canvas covers
    // the viewport in colour that has not been through that pass and the
    // washed background behind it never shows. Left transparent elsewhere,
    // where the page beneath is drawn correctly and showing through it is the
    // point.
    alpha: !/\bAndroid\b/.test(navigator.userAgent),
  });
  if (!gl) return null;

  // One program per style, built when that style is first asked for and kept:
  // switching back and forth in Settings should not recompile a shader every
  // time, and five of them is a few kilobytes of driver state.
  const programs = new Map();
  const vs = compile(gl, gl.VERTEX_SHADER, VERTEX);
  if (!vs) return null;

  const programFor = (id) => {
    if (programs.has(id)) return programs.get(id);
    const fs = compile(gl, gl.FRAGMENT_SHADER, fragmentFor(id));
    if (!fs) return programs.get("blobs") ?? null;
    const p = gl.createProgram();
    gl.attachShader(p, vs);
    gl.attachShader(p, fs);
    gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) {
      console.warn("backdrop link failed:", gl.getProgramInfoLog(p));
      return null;
    }
    programs.set(id, p);
    return p;
  };

  let styleId = backdropSettings().style ?? "blobs";
  const prog = programFor(styleId);
  if (!prog) return null;
  gl.useProgram(prog);

  // One triangle covering the viewport. A triangle rather than two for a quad:
  // fewer vertices and no seam down the diagonal.
  const buf = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, buf);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
  // Uniform locations belong to a program, so they are looked up per program
  // rather than once: a location from one is meaningless in another, and the
  // symptom of getting that wrong is a backdrop that goes black on the second
  // style you try.
  let active = prog;
  const bind = (p) => {
    gl.useProgram(p);
    const loc = gl.getAttribLocation(p, "pos");
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    return {
      size: gl.getUniformLocation(p, "u_size"),
      time: gl.getUniformLocation(p, "u_time"),
      low: gl.getUniformLocation(p, "u_low"),
      mid: gl.getUniformLocation(p, "u_mid"),
      high: gl.getUniformLocation(p, "u_high"),
      strength: gl.getUniformLocation(p, "u_strength"),
      speed: gl.getUniformLocation(p, "u_speed"),
    };
  };
  let u = bind(prog);

  // Declared before `apply`, which calls it.
  //
  // It was the other way round for one version, and `const` bindings are in
  // the dead zone until their line runs — so the first call threw
  // ReferenceError, out of startBackdrop, out of the init that called it, and
  // took the tab row, the settings button and everything else after it with
  // it. A backdrop is decoration; it managed to be fatal.
  let sized = false;
  const resize = () => {
    // Half resolution: this is out-of-focus noise, and full-resolution costs
    // four times the pixels for something nobody is looking directly at.
    const scale = 0.5;
    canvas.width = Math.max(2, Math.floor(window.innerWidth * scale));
    canvas.height = Math.max(2, Math.floor(window.innerHeight * scale));
    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.useProgram(active);
    gl.uniform2f(u.size, canvas.width, canvas.height);
    sized = true;
  };

  const apply = (cfg) => {
    const want = cfg.style ?? "blobs";
    if (want !== styleId) {
      const next = programFor(want);
      if (next) {
        styleId = want;
        active = next;
        u = bind(next);
        sized = false;
      }
    }
    // A preset supplies the pair; "custom" falls through to the user's own, and
    // an unset custom color falls through again to the theme's — so the
    // default follows the palette rather than being a second place to maintain.
    const { low, mid, high } = presetColors(cfg);
    gl.useProgram(active);
    // A two-color scheme has no middle, so the midpoint of the pair stands in
    // and the ramp behaves exactly as the old two-stop mix did.
    const lo = rgb(low, "--bg", [0.05, 0.05, 0.07]);
    const hi = rgb(high, "--accent", [0.18, 0.2, 0.36]);
    const md = mid ? rgb(mid, "--accent", hi) : lo.map((v, i) => (v + hi[i]) / 2);
    gl.uniform3fv(u.low, lo);
    gl.uniform3fv(u.mid, md);
    gl.uniform3fv(u.high, hi);
    gl.uniform1f(u.strength, cfg.strength);
    // Scaled by the style's own pace, not handed over raw: one number on the
    // slider has to mean the same amount of movement whichever shape is drawing.
    gl.uniform1f(u.speed, cfg.speed * (backdropStyle(styleId).pace ?? 1));
    if (!sized) resize();
  };
  apply(backdropSettings());
  live = apply;

  resize();
  window.addEventListener("resize", resize);

  let frame = 0;
  let stopped = false;
  const start = performance.now();
  let lastDraw = 0;

  // A drifting gradient at 30fps and the same gradient at 120fps are the same
  // picture. On a ProMotion display the loop was running four times faster than
  // anything in it changes, which is four times the GPU for no difference —
  // and this thing is on screen the entire time the app is open.
  //
  // The rate is a setting now; 30 is only the desktop's default. Read every
  // frame rather than captured, so changing it in Settings takes effect on the
  // next one rather than on the next restart.
  //
  // Zero means a still picture: the shader is drawn once, at its opening state,
  // and then never again. The frame callback keeps being scheduled — an empty
  // callback costs nothing next to a GL draw — so turning motion back on does
  // not need the loop restarting.
  let drawnOnce = false;

  const draw = (now) => {
    if (stopped) return;
    frame = requestAnimationFrame(draw);
    const fps = backdropFps();
    if (fps <= 0) {
      if (drawnOnce) return;
    } else if (now - lastDraw < 1000 / fps) {
      return;
    }
    drawnOnce = true;
    lastDraw = now;
    gl.useProgram(active);
    gl.uniform1f(u.time, (now - start) / 1000);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  };

  // Nothing to draw for while the window is not on screen. WebKit does not
  // reliably throttle an occluded window's animation frames here, so an app
  // left open behind something else kept a shader running at full rate for as
  // long as it was open — which is most of the day.
  const onVisibility = () => {
    if (document.hidden) {
      cancelAnimationFrame(frame);
    } else if (!stopped) {
      // Reset the clock so the gradient carries on from where it was rather
      // than jumping forward by however long the window was covered.
      lastDraw = 0;
      frame = requestAnimationFrame(draw);
    }
  };
  document.addEventListener("visibilitychange", onVisibility);

  document.body.prepend(canvas);
  // Without this the page's own opaque background sits on top of the canvas and
  // the shader renders perfectly where nobody can see it.
  document.documentElement.classList.add("backdrop-on");
  frame = requestAnimationFrame(draw);

  running = () => {
    stopped = true;
    cancelAnimationFrame(frame);
    document.removeEventListener("visibilitychange", onVisibility);
    window.removeEventListener("resize", resize);
    canvas.remove();
    document.documentElement.classList.remove("backdrop-on");
    live = null;
    running = null;
  };
  return running;
}

export function stopBackdrop() {
  if (running) running();
}


/// Whether this machine can run it at all, without starting anything.
///
/// Used by Settings to say "not supported here" rather than offering a switch
/// that silently does nothing.
export function backdropSupported() {
  try {
    return !!document.createElement("canvas").getContext("webgl2");
  } catch {
    return false;
  }
}

// ---- Glass tint -----------------------------------------------------------
//
// Vista's "Window Color and Appearance" let you pick the color of the glass,
// and that choice is most of why two Vista machines looked different from each
// other. Same idea: one color drives the bars, the button gel, the hover glow
// and the focus ring, because in Aero they were all the same light.

/// One palette for both surfaces.
///
/// The glass tint and the shader backdrop were two dropdowns of seven and eight
/// colors, chosen separately, and every sensible combination was a pair that
/// already matched — "Aero blue" glass over the "Midnight" backdrop, "Jade"
/// over "Moss". Two controls whose only correct settings are a diagonal of the
/// grid they span is one control.
///
/// `glass` tints the cards, the selection glow and the controls; `low` and
/// `high` are the two ends of the gradient the shader draws. Custom keeps all
/// three separately settable, because someone who wants an unmatched pair
/// should still be able to have one.
export const SCHEMES = [
  { id: "midnight", label: "Midnight", glass: "#4d8fd6", low: "#0b0d16", high: "#2a3566" },
  { id: "frost",    label: "Frost",    glass: "#8fb8d8", low: "#0b0f14", high: "#33506b" },
  { id: "abyss",    label: "Abyss",    glass: "#3aa0b5", low: "#06090c", high: "#12414d" },
  { id: "moss",     label: "Moss",     glass: "#3f9e86", low: "#0a1210", high: "#1f4a37" },
  { id: "ember",    label: "Ember",    glass: "#c8873c", low: "#140b09", high: "#5c2418" },
  { id: "rust",     label: "Rust",     glass: "#b06a35", low: "#150f09", high: "#5e3a17" },
  { id: "wine",     label: "Wine",     glass: "#b04a55", low: "#130a0e", high: "#54203a" },
  { id: "plum",     label: "Plum",     glass: "#7b62c4", low: "#120a16", high: "#452b5e" },
  { id: "slate",    label: "Slate",    glass: "#6d7681", low: "#0f1113", high: "#333a42" },
  { id: "custom",   label: "Custom",   glass: null,      low: null,      high: null },
];

/// Three-color schemes, kept apart from the pairs above.
///
/// A gradient through a third color is a different thing from a tint between
/// two: `sunset` is not "orange with more orange", it is red through orange to
/// yellow, and the middle is what makes it read as a sweep. Named after what
/// they look like rather than after a vendor's lighting preset.
export const TRIPLES = [
  { id: "sunset",   label: "Sunset",   glass: "#e0794a", low: "#150a12", mid: "#7a2540", high: "#e8a24a" },
  { id: "vapor",    label: "Vapourwave", glass: "#ff77c8", low: "#160a2a", mid: "#8a2b8f", high: "#39d7e8" },
  { id: "aurora3",  label: "Aurora",   glass: "#54d6a0", low: "#04120f", mid: "#166b57", high: "#8ef0c0" },
  { id: "ember3",   label: "Furnace",  glass: "#ff8a3d", low: "#160604", mid: "#8f2a09", high: "#ffc44d" },
  { id: "ocean",    label: "Deep water", glass: "#3fa9d8", low: "#03080f", mid: "#0d3a5e", high: "#63d0f0" },
  { id: "spectrum", label: "Spectrum", glass: "#7c8cff", low: "#2a0d4a", mid: "#0d4a86", high: "#3fbf8f" },
  { id: "magma",    label: "Magma",    glass: "#e8543d", low: "#0a0406", mid: "#5e1020", high: "#ff9a3c" },
  { id: "toxic",    label: "Toxic",    glass: "#9ee84a", low: "#07110a", mid: "#2f6b17", high: "#d4ff5c" },
];

/// Full-spectrum sweeps, the kind RGB lighting software ships as presets.
///
/// These run right round the hue circle rather than between two ends of it,
/// which the three-stop ramp can only approximate — low, mid and high are
/// picked so the two halves of the ramp cover opposite sides of the wheel and
/// the result reads as a rainbow rather than as a gradient.
export const RAINBOWS = [
  // Not the three raw primaries. #e02020 / #20c040 / #2060e0 is a television
  // test card: fully saturated red, green and blue have no common ground, so
  // the ramp between them passes through mud rather than through a hue sweep.
  // Pulled off the corners and matched in lightness, which is what makes the
  // sweep read as one thing.
  { id: "rgb",      label: "RGB",        glass: "#5f8fe0", low: "#e2445c", mid: "#3fc98a", high: "#5a7fe8" },
  { id: "prism",    label: "Prism",      glass: "#8f5fd8", low: "#c01ad0", mid: "#20b0e0", high: "#e8d020" },
  { id: "neon",     label: "Neon",       glass: "#ff4fd8", low: "#ff2bd6", mid: "#00e5ff", high: "#b026ff" },
  { id: "candy",    label: "Candy",      glass: "#ff7ab8", low: "#ff5fa2", mid: "#ffd166", high: "#5fd3ff" },
  { id: "heat",     label: "Heat map",   glass: "#ff9a3c", low: "#1020a0", mid: "#20c060", high: "#ff3020" },
  { id: "pastel",   label: "Pastel",     glass: "#b8a6e8", low: "#f2a6c2", mid: "#a6e8c8", high: "#a6c2f2" },
];

/// Every scheme the dropdown offers, pairs first.
export const ALL_SCHEMES = [
  ...SCHEMES.filter((s) => s.id !== "custom"),
  ...TRIPLES,
  ...RAINBOWS,
  SCHEMES.find((s) => s.id === "custom"),
];

/// The groups the dropdown draws, so the list of forty is browsable.
export const SCHEME_GROUPS = [
  ["Pairs", SCHEMES.filter((s) => s.id !== "custom")],
  ["Spectrums", TRIPLES],
  ["Rainbows", RAINBOWS],
  ["Your own", [SCHEMES.find((s) => s.id === "custom")]],
];


const GLASS_KEY = "glassTint";
const TINT_KEY = "glassStrength";

/// How opaque the glass is, as a percentage — every card, the selected row,
/// the cover art behind a game, and the preview pane, which is one of them.
///
/// This is the transparency control, and calling it "tint strength" was the
/// reason nobody could find it. `--tint` is the *opacity* the surfaces mix
/// their color in at and it has never been anything else: there are five
/// `var(--tint)` in the stylesheet and all five are the percentage in a
/// `color-mix` against `transparent`. So the preview pane's own second slider
/// was a second control over one idea, which is how the two surfaces came to
/// disagree. One number, every sheet of glass in the window.
///
/// Read by asking whether the key is there rather than what it coerces to:
/// `n > 0` kept an unset key from reading as zero, but it also threw away a
/// deliberate zero, so dragging the glass to clear put it back to 18 on the
/// next start.
/// How often the backdrop redraws, in frames a second.
///
/// The steps are a ladder rather than a slider: the difference between 28 and
/// 30 is nothing anybody can see, and the choices that matter are "off",
/// "barely moving", and a handful of real rates.
export const BACKDROP_FPS_STEPS = [0, 1, 5, 10, 15, 20, 30, 60];

const FPS_KEY = "backdrop.fps";

/// The default is not the same on both, and that is the point.
///
/// A drifting gradient at 30fps and the same gradient at 120 are the same
/// picture, so 30 was already the cap on the desktop. A handheld is a battery
/// and a small chip drawing this behind everything for as long as the app is
/// open, so it starts at 10 — still motion, a third of the work.
function defaultFps() {
  return /\bAndroid\b/.test(navigator.userAgent) ? 10 : 30;
}

export function backdropFps() {
  const raw = localStorage.getItem(FPS_KEY);
  if (raw === null) return defaultFps();
  const n = Number(raw);
  return BACKDROP_FPS_STEPS.includes(n) ? n : defaultFps();
}

/// Set the rate, snapping to the nearest step.
export function setBackdropFps(fps, { announce = true } = {}) {
  const n = Number(fps);
  const value = BACKDROP_FPS_STEPS.includes(n)
    ? n
    : BACKDROP_FPS_STEPS.reduce((a, b) => (Math.abs(b - n) < Math.abs(a - n) ? b : a));
  localStorage.setItem(FPS_KEY, String(value));
  if (announce) window.__TAURI__?.event?.emit?.("backdrop-fps", value);
  return value;
}

export function glassStrength() {
  const raw = localStorage.getItem(TINT_KEY);
  if (raw === null) return 18;
  const n = Number(raw);
  return Number.isFinite(n) && n >= 0 ? Math.min(60, n) : 18;
}

export function setGlassStrength(pct, { announce = true } = {}) {
  const value = Math.max(0, Math.min(60, Math.round(Number(pct) || 0)));
  document.documentElement.style.setProperty("--tint", `${value}%`);
  if (announce) {
    localStorage.setItem(TINT_KEY, String(value));
    window.__TAURI__?.event?.emit?.("glass-strength", value);
  }
  return value;
}

export function glassTint() {
  // The first scheme's glass when nothing has been chosen. This read
  // GLASS_PRESETS, which was deleted when the two color dropdowns were merged
  // — so on a machine with no stored tint, which is every new install, the
  // first call threw before anything had been painted.
  return localStorage.getItem(GLASS_KEY) || SCHEMES[0].glass;
}

/// Apply the tint to this document, and tell the other window.
///
/// Both windows want it: the library has the cards, Settings has its own
/// controls. A tint applied in one and not the other is worse than no tint,
/// because it looks like a bug.
export function setGlassTint(color, { announce = true } = {}) {
  const value = /^#[0-9a-f]{6}$/i.test(color) ? color : SCHEMES[0].glass;
  document.documentElement.style.setProperty("--glass", value);
  // The same colour as three numbers, for browsers with no `color-mix()`.
  //
  // Every tinted surface in the stylesheet is some percentage of this colour,
  // written as a mix with `transparent`. An engine that does not know the
  // function throws the whole declaration away — which is Chromium 109, which
  // is what the Thor has baked into its system image and cannot update. The
  // fallback rules say `rgb(var(--glass-rgb) / 40%)` instead, and that needs
  // the components rather than the hex.
  document.documentElement.style.setProperty(
    "--glass-rgb",
    `${parseInt(value.slice(1, 3), 16)} ${parseInt(value.slice(3, 5), 16)} ${parseInt(value.slice(5, 7), 16)}`
  );
  if (announce) {
    localStorage.setItem(GLASS_KEY, value);
    window.__TAURI__?.event?.emit?.("glass-tint", value);
  }
  return value;
}

/// Called at startup in every window with bars and cards to tint.
export function applyStoredGlassTint() {
  setGlassTint(glassTint(), { announce: false });
  setGlassStrength(glassStrength(), { announce: false });
}

