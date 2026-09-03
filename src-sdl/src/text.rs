// Drawing words.
//
// The item docs/handheld-frontend.md calls "the least glamorous here and the
// one most likely to make the result feel cheap", and it is right. A webview
// wraps, ellipsises and falls back across fonts without being asked. Nothing
// below that does any of it for free.
//
// Four separate problems, and skipping any one of them shows:
//
//   * **Shaping.** Turning characters into positioned glyphs. Trivial for
//     English, not for anything with marks or joining forms.
//   * **Fallback.** A library with Japanese titles in it will ask for glyphs
//     no Latin face has. Drawing a row of empty boxes is the failure everyone
//     has seen and nobody reports as a bug, because it looks deliberate.
//   * **Line breaking.** Where a line may be split, which is Unicode's answer
//     and not "at spaces" — CJK has no spaces and breaks almost anywhere.
//   * **Ellipsis.** A title too long for its card has to stop somewhere, and
//     the somewhere has to be a character boundary in the original string, not
//     a convenient-looking gap between glyphs.
//
// `cosmic-text` answers the first three. The fourth is here, because where a
// name gets cut is a decision about this app rather than about Unicode.

use anyhow::Result;
use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, Wrap};
use moose_rack::script::{self, Script};
use crate::gfx::{Gfx, Rgba, Texture};
use moose_rack::layout::Rect;
use std::collections::{BTreeMap, HashMap};

/// What a rendered piece of text is asked for.
///
/// Everything in points. The scale turns them into pixels at the last moment,
/// which is also why it is part of the key: the same title at the same size on
/// a different display is a different set of pixels.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Spec {
    pub text: String,
    /// Point size, times a hundred, so the key can be hashed. Nobody asks for
    /// a hundredth of a point.
    size: u32,
    /// The box it has to fit inside, in pixels, or `None` for "as wide as it
    /// likes".
    width: Option<u32>,
    /// How many lines it may take before it is cut short.
    lines: u16,
    scale: u32,
}

impl Spec {
    pub fn new(text: impl Into<String>, size_points: f32, scale: f32) -> Self {
        Spec {
            text: text.into(),
            size: (size_points * 100.0).round() as u32,
            width: None,
            lines: 1,
            scale: (scale * 100.0).round() as u32,
        }
    }

    /// Wrap inside `width` points, over at most `lines` of them.
    pub fn wrapped(mut self, width_points: f32, lines: u16) -> Self {
        self.width = Some((width_points * 100.0).round() as u32);
        self.lines = lines.max(1);
        self
    }

    fn size_px(&self) -> f32 {
        self.size as f32 / 100.0 * self.scale as f32 / 100.0
    }

    fn width_px(&self) -> Option<f32> {
        self.width.map(|w| w as f32 / 100.0 * self.scale as f32 / 100.0)
    }
}

/// A laid-out, rasterised piece of text: coverage, and how big it came out.
///
/// Grayscale rather than color, because the color is decided by whatever
/// draws it — a name is dim in one place and bright in another, and
/// rasterising it twice for that would be silly.
pub struct Raster {
    pub width: u32,
    pub height: u32,
    /// One byte of coverage per pixel.
    pub coverage: Vec<u8>,
    /// Whether the text was cut short to fit.
    pub clipped: bool,
}

/// The key `chosen` files the everyday face under. Not a language tag,
/// because "Latin, Cyrillic, Greek and the rest" is not a language.
const PLAIN: &str = "*";

/// Where the fonts we ship live, if they are anywhere.
///
/// Three places, in the order they become true: beside the executable once it
/// is installed, inside a macOS bundle, and in the source tree while somebody
/// is working on it. None of them existing is survivable — the machine's own
/// faces still answer — so this returns an option rather than failing.
fn bundled_fonts() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok();
    let beside = exe.as_ref().and_then(|p| p.parent()).map(|p| p.join("fonts"));
    let in_bundle = exe
        .as_ref()
        .and_then(|p| p.parent()?.parent())
        .map(|p| p.join("Resources").join("fonts"));
    let in_tree = Some(std::path::PathBuf::from("assets/fonts"));
    [beside, in_bundle, in_tree].into_iter().flatten().find(|p| p.is_dir())
}

/// What a line is cut short with. One character, not three dots: the ellipsis
/// is narrower, and it is what every other list in the world uses.
const ELLIPSIS: char = '…';

pub struct Fonts {
    system: FontSystem,
    swash: SwashCache,
    /// Which installed family to use for each writing system, worked out once.
    ///
    /// Asking the database on every string would be a lookup per label per
    /// frame for an answer that changes when somebody installs a font.
    chosen: BTreeMap<&'static str, Option<String>>,
}

/// Which family to draw this string in, if it is one where that matters.
///
/// The whole of the Han unification fix: `script::of` reads the title and says
/// Japanese, Korean, simplified or traditional, and this asks for the family
/// that draws those forms. Everything else gets the generic sans-serif,
/// because covering the script is the whole of the answer for it.
fn attrs_for<'a>(family: Option<&'a str>) -> Attrs<'a> {
    match family {
        Some(name) => Attrs::new().family(Family::Name(name)),
        None => Attrs::new().family(Family::SansSerif),
    }
}

impl Fonts {
    /// Find the faces the machine has.
    ///
    /// The same directories fontconfig reads, which is how the handheld's
    /// `fonts-noto-cjk` is found without us shipping or naming it — see
    /// docs/handheld-device.md.
    pub fn load() -> Result<Self> {
        let mut system = FontSystem::new();
        // The faces we ship, on top of whatever the machine has. On the
        // handheld this is the only Latin face there is: the rootfs installs
        // `fonts-noto-cjk` and nothing else, and EmulationStation was quietly
        // fetching Android's Droid fallback to cover the gap.
        if let Some(dir) = bundled_fonts() {
            system.db_mut().load_fonts_dir(&dir);
        }
        let mut fonts = Fonts { system, swash: SwashCache::new(), chosen: BTreeMap::new() };
        if fonts.faces() == 0 {
            anyhow::bail!("no fonts on this machine at all");
        }
        for script in [
            Script::Plain,
            Script::Japanese,
            Script::Korean,
            Script::Simplified,
            Script::Traditional,
        ] {
            let installed = script
                .families()
                .iter()
                .find(|wanted| fonts.has_family(wanted))
                .map(|name| (*name).to_owned());
            fonts.chosen.insert(script.language_tag().unwrap_or(PLAIN), installed);
        }
        Ok(fonts)
    }

    fn has_family(&self, name: &str) -> bool {
        self.system
            .db()
            .faces()
            .any(|face| face.families.iter().any(|(family, _)| family == name))
    }

    /// The family this string should be drawn in, or `None` for "anything that
    /// covers it".
    pub fn family_for(&self, text: &str) -> Option<&str> {
        let script = script::of(text);
        let key = script.language_tag().unwrap_or(PLAIN);
        self.chosen.get(key)?.as_deref()
    }

    pub fn faces(&self) -> usize {
        self.system.db().len()
    }

    /// Whether anything installed can draw this string.
    ///
    /// Not a nicety: on a machine with no CJK face a Japanese title silently
    /// becomes a row of boxes, and there is nothing on screen to say the font
    /// is missing rather than the name being wrong.
    pub fn can_draw(&mut self, text: &str) -> bool {
        let laid = self.lay_out(&Spec::new(text, 16.0, 1.0));
        laid.glyphs > 0 && laid.missing == 0
    }

    /// Which face was actually chosen to draw this string.
    ///
    /// A diagnostic, and a sharp one: fallback picking *a* face that has the
    /// glyph is not the same as picking the right one.
    ///
    /// **Han unification is the trap.** Chinese, Japanese and Korean share
    /// code points for characters whose correct *shapes* differ — 直, 骨, 話,
    /// 令 are drawn differently in each, and a reader of one notices the other
    /// immediately. Fallback here picks a family by which scripts it covers,
    /// not by what language the text is in, so on the handheld — where the one
    /// installed family is `fonts-noto-cjk`, covering all four with variants
    /// chosen by *language tag* — Chinese titles will come out in Japanese
    /// shapes, or the other way round, depending on nothing but which variant
    /// fontconfig happens to list first.
    ///
    /// It is not a bug anybody reports, because the text is perfectly legible.
    /// It just looks foreign.
    ///
    /// cosmic-text 0.19 takes its shaping language from the system locale
    /// rather than per string (`shape.rs`, `buffer.language()`), and `Attrs`
    /// has no language of its own. The lever we do have is the family: a title
    /// known to be Chinese can ask for `Family::Name("Noto Sans CJK SC")`
    /// outright. What is missing is knowing that it *is* Chinese — the ROM's
    /// region is in the library metadata, and that is where the answer will
    /// come from. See docs/handheld-frontend.md.
    pub fn face_for(&mut self, text: &str) -> Option<String> {
        let spec = Spec::new(text, 16.0, 1.0);
        let size = spec.size_px();
        let mut buffer = Buffer::new(&mut self.system, Metrics::new(size, size * 1.3));
        let family = self.family_for(text).map(str::to_owned);
        let id = {
            let mut buffer = buffer.borrow_with(&mut self.system);
            buffer.set_text(text, &attrs_for(family.as_deref()), Shaping::Advanced, None);
            buffer.shape_until_scroll(false);
            buffer.layout_runs().flat_map(|r| r.glyphs.iter()).map(|g| g.font_id).next()?
        };
        self.system.db().face(id).map(|f| {
            f.families.first().map(|(name, _)| name.clone()).unwrap_or_else(|| f.post_script_name.clone())
        })
    }

    fn lay_out(&mut self, spec: &Spec) -> Laid {
        let family = self.family_for(&spec.text).map(str::to_owned);
        let size = spec.size_px();
        // Line height at 1.3x. Tighter and diacritics collide with the line
        // above; looser and a two-line title reads as two titles.
        let mut buffer = Buffer::new(&mut self.system, Metrics::new(size, size * 1.3));
        let mut buffer = buffer.borrow_with(&mut self.system);
        buffer.set_wrap(if spec.width_px().is_some() { Wrap::WordOrGlyph } else { Wrap::None });
        buffer.set_size(spec.width_px(), None);
        // `Shaping::Advanced` rather than `Basic`: basic skips the shaper,
        // which is fine for English and wrong for everything else.
        buffer.set_text(&spec.text, &attrs_for(family.as_deref()), Shaping::Advanced, None);
        buffer.shape_until_scroll(false);

        let mut glyphs = 0usize;
        let mut missing = 0usize;
        let mut width = 0f32;
        let mut lines = 0usize;
        // Where the last line we are allowed to draw runs past its box.
        let mut cut: Option<usize> = None;
        for run in buffer.layout_runs() {
            lines += 1;
            width = width.max(run.line_w);
            for glyph in run.glyphs {
                glyphs += 1;
                if glyph.glyph_id == 0 {
                    missing += 1;
                }
            }
            if lines == spec.lines as usize {
                cut = run.glyphs.last().map(|g| g.end);
            }
        }
        Laid { glyphs, missing, width, lines, cut }
    }

    /// Lay `spec` out and draw it, cutting it short if it does not fit.
    pub fn render(&mut self, spec: &Spec) -> Raster {
        let laid = self.lay_out(spec);
        let allowed = spec.lines as usize;
        if laid.lines <= allowed {
            return self.raster(&spec.text, spec, false);
        }

        // Too tall. Cut at the end of the last line we are allowed and put an
        // ellipsis there, then lay it out again — once, and only for the
        // strings that actually overflow.
        //
        // Cut on a byte offset the shaper gave us, which is a character
        // boundary in the *original string*. Dropping glyphs instead would cut
        // inside a cluster, and half of a composed character is not a
        // character.
        let cut = laid.cut.unwrap_or(spec.text.len()).min(spec.text.len());
        let head = spec.text[..cut].trim_end();
        let shortened = trim_to_fit(self, spec, head);
        self.raster(&shortened, spec, true)
    }

    /// Take characters off the end until the ellipsis fits too.
    ///
    /// The last line was full before the ellipsis was added to it, so adding
    /// one pushes it onto a line of its own — an ellipsis alone on the third
    /// line of a two-line title, which is worse than the overflow.
    fn raster(&mut self, text: &str, spec: &Spec, clipped: bool) -> Raster {
        let family = self.family_for(text).map(str::to_owned);
        let size = spec.size_px();
        let mut buffer = Buffer::new(&mut self.system, Metrics::new(size, size * 1.3));
        {
            let mut buffer = buffer.borrow_with(&mut self.system);
            buffer.set_wrap(if spec.width_px().is_some() { Wrap::WordOrGlyph } else { Wrap::None });
            buffer.set_size(spec.width_px(), None);
            buffer.set_text(text, &attrs_for(family.as_deref()), Shaping::Advanced, None);
            buffer.shape_until_scroll(false);
        }

        let mut width = 0f32;
        let mut lines = 0usize;
        for run in buffer.layout_runs() {
            width = width.max(run.line_w);
            lines += 1;
        }
        // Where the glyphs actually landed, rather than where the box is.
        //
        // Not the same thing, and the difference is not a rounding error: with
        // a wrap width set, a right-to-left line is laid out from the *right*
        // edge of that width. Sizing the image to the ink's width and blitting
        // from zero put every Arabic glyph outside its own bitmap — a face was
        // found, twelve glyphs were shaped, and nothing at all was drawn.
        let runs: Vec<_> = buffer.layout_runs().map(|r| (r.line_y, r.glyphs.to_vec())).collect();
        let mut placed = Vec::new();
        let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
        for (line_y, glyphs) in &runs {
            for glyph in glyphs {
                let physical = glyph.physical((0.0, 0.0), 1.0);
                let Some(image) = self.swash.get_image(&mut self.system, physical.cache_key).clone()
                else {
                    continue;
                };
                let left = physical.x + image.placement.left;
                min_x = min_x.min(left as f32);
                max_x = max_x.max((left + image.placement.width as i32) as f32);
                placed.push((left, physical.y + *line_y as i32 - image.placement.top, image));
            }
        }
        if placed.is_empty() {
            min_x = 0.0;
            max_x = width.max(1.0);
        }
        // Slid over so the leftmost ink sits at zero. The image is then the
        // size of what is in it, whichever direction the script runs.
        let shift = min_x as i32;
        let ink_w = (max_x - min_x).ceil().max(1.0);
        let w = spec.width_px().map(|box_w| ink_w.min(box_w)).unwrap_or(ink_w).ceil().max(1.0);
        // Height stays a whole number of line boxes rather than the ink's own
        // extent: a label whose height depended on whether its letters had
        // descenders would sit at a different height on every card.
        let h = (size * 1.3 * lines.max(1) as f32).ceil().max(1.0);
        let (w, h) = (w as u32, h as u32);
        let mut coverage = vec![0u8; (w as usize) * (h as usize)];
        for (x, y, image) in &placed {
            blit(&mut coverage, w, h, x - shift, *y, image);
        }
        Raster { width: w, height: h, coverage, clipped }
    }
}

/// Put the ellipsis on, shortening until it fits on the lines allowed.
fn trim_to_fit(fonts: &mut Fonts, spec: &Spec, head: &str) -> String {
    let mut head = head.to_owned();
    for _ in 0..64 {
        let candidate = format!("{head}{ELLIPSIS}");
        let probe = Spec { text: candidate.clone(), ..spec.clone() };
        if fonts.lay_out(&probe).lines <= spec.lines as usize {
            return candidate;
        }
        match shorten_once(&head) {
            Some(shorter) if !shorter.is_empty() => head = shorter,
            _ => break,
        }
    }
    format!("{head}{ELLIPSIS}")
}

/// One character off the end, and any space that character was hiding behind.
fn shorten_once(head: &str) -> Option<String> {
    let mut chars: Vec<char> = head.chars().collect();
    chars.pop()?;
    Some(chars.into_iter().collect::<String>().trim_end().to_owned())
}

struct Laid {
    glyphs: usize,
    /// Glyphs no installed face could draw. Zero, or the text is boxes.
    missing: usize,
    #[allow(dead_code)]
    width: f32,
    lines: usize,
    cut: Option<usize>,
}

/// One glyph's coverage into the page, clipped at every edge.
fn blit(page: &mut [u8], w: u32, h: u32, x: i32, y: i32, image: &cosmic_text::SwashImage) {
    let gw = image.placement.width as i32;
    let gh = image.placement.height as i32;
    for row in 0..gh {
        let py = y + row;
        if py < 0 || py >= h as i32 {
            continue;
        }
        for col in 0..gw {
            let px = x + col;
            if px < 0 || px >= w as i32 {
                continue;
            }
            let from = (row * gw + col) as usize;
            let Some(&value) = image.data.get(from) else { continue };
            if value == 0 {
                continue;
            }
            let to = py as usize * w as usize + px as usize;
            // Lighten rather than overwrite: glyphs overlap at their edges and
            // a plain copy leaves a seam through the join.
            page[to] = page[to].max(value);
        }
    }
}

/// Rendered text, kept as textures so a name is only shaped once.
///
/// Titles repeat: the same card is drawn every frame, and the same name comes
/// back every time the list is scrolled past. Shaping and rasterising are the
/// expensive half of this module and neither answer ever changes, so the work
/// is done once and the result is a texture.
///
/// The texture is white with the coverage as its alpha, tinted at draw time.
/// One raster then serves a name however it is colored — dim in the metadata
/// line, bright under the cursor — instead of one per color.
pub struct Painter {
    fonts: Fonts,
    cache: HashMap<Spec, Drawn>,
}

struct Drawn {
    texture: Texture,
    width: u32,
    height: u32,
}

/// Kept small on purpose while there is nothing to evict for. A real cursor
/// through 2,506 games wants a least-recently-used bound; this wants to not
/// grow without limit before that exists.
const CACHE_LIMIT: usize = 512;

impl Painter {
    pub fn new() -> Result<Self> {
        Ok(Painter { fonts: Fonts::load()?, cache: HashMap::new() })
    }

    pub fn faces(&self) -> usize {
        self.fonts.faces()
    }

    /// The family this string asked for, or `None` where covering the script
    /// is the whole of the answer.
    pub fn family_for(&self, text: &str) -> Option<&str> {
        self.fonts.family_for(text)
    }

    /// Which face was chosen to draw this string. See [`Fonts::face_for`] for
    /// why the answer is worth looking at and not only whether there was one.
    pub fn face_for(&mut self, text: &str) -> Option<String> {
        self.fonts.face_for(text)
    }

    /// Whether anything installed can draw this string.
    pub fn can_draw(&mut self, text: &str) -> bool {
        self.fonts.can_draw(text)
    }

    /// Lay a label out and say whether it had to be cut to fit.
    pub fn is_clipped(&mut self, spec: &Spec) -> bool {
        self.fonts.render(spec).clipped
    }

    /// How big this text comes out, in pixels, without drawing it.
    pub fn measure(&mut self, gfx: &Gfx, spec: &Spec) -> (u32, u32) {
        let drawn = self.entry(gfx, spec);
        (drawn.width, drawn.height)
    }

    /// Draw it at the top left of a box, and say how tall it came out.
    ///
    /// The box rather than two numbers, so a caller never adds a gap to an
    /// offset — and the height back, because what goes under a label depends
    /// on how many lines it took.
    pub fn put(&mut self, gfx: &Gfx, spec: &Spec, at: Rect, color: Rgba) -> f32 {
        let (_, h) = self.measure(gfx, spec);
        self.draw(gfx, spec, at.x, at.y, color);
        h as f32
    }

    /// The same, pushed to the right-hand edge of the box.
    pub fn put_right(&mut self, gfx: &Gfx, spec: &Spec, at: Rect, color: Rgba) {
        let (w, _) = self.measure(gfx, spec);
        self.draw(gfx, spec, at.right() - w as f32, at.y, color);
    }

    /// The same, centered in it.
    pub fn put_centered(&mut self, gfx: &Gfx, spec: &Spec, at: Rect, color: Rgba) {
        let (w, h) = self.measure(gfx, spec);
        let box_ = at.center(w as f32, h as f32);
        self.draw(gfx, spec, box_.x, box_.y, color);
    }

    /// Draw it, with its top-left corner at `x`, `y` in pixels.
    ///
    /// Snapped to whole pixels, and that is the whole of this comment's
    /// reason for existing.
    ///
    /// The glyphs are rasterised onto a whole-pixel grid — `physical()` returns
    /// integers — and then the finished bitmap was being placed at whatever the
    /// layout worked out, which is fractional almost everywhere: a twelve-column
    /// split does not divide evenly, and centring divides by two and lands on a
    /// half by construction. A texture drawn half a pixel off is sampled across
    /// two pixels at every edge, so every stem of every letter came out as two
    /// half-lit columns instead of one lit one.
    ///
    /// That is what "the whole screen is blurry" was. Not the resolution — the
    /// panel is 640x480 and the app draws 640x480 — and not the type size. A
    /// half-pixel, everywhere, on everything.
    pub fn draw(&mut self, gfx: &Gfx, spec: &Spec, x: f32, y: f32, color: Rgba) {
        let (w, h) = {
            let drawn = self.entry(gfx, spec);
            (drawn.width, drawn.height)
        };
        let Some(drawn) = self.cache.get(spec) else { return };
        gfx.image(
            &drawn.texture,
            x.round(),
            y.round(),
            w as f32,
            h as f32,
            color,
        );
    }

    fn entry(&mut self, gfx: &Gfx, spec: &Spec) -> &Drawn {
        if !self.cache.contains_key(spec) {
            if self.cache.len() >= CACHE_LIMIT {
                self.cache.clear();
            }
            let raster = self.fonts.render(spec);
            let texture = gfx.upload_coverage(raster.width, raster.height, &raster.coverage);
            self.cache.insert(
                spec.clone(),
                Drawn { texture, width: raster.width, height: raster.height },
            );
        }
        &self.cache[spec]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fonts() -> Fonts {
        Fonts::load().expect("a machine with no fonts cannot run this test")
    }

    #[test]
    fn a_machine_has_fonts_and_we_find_them() {
        assert!(fonts().faces() > 0);
    }

    /// The failure nobody reports: a Japanese title in a Latin face is a row
    /// of empty boxes, and it looks deliberate.
    #[test]
    fn japanese_finds_a_face_that_can_draw_it() {
        let mut f = fonts();
        assert!(f.can_draw("ゼルダの伝説"), "no CJK face, or fallback is not working");
        assert!(f.can_draw("スーパーマリオブラザーズ"));
    }

    #[test]
    fn so_does_the_alphabet_it_was_already_good_at() {
        let mut f = fonts();
        assert!(f.can_draw("Super Mario Bros."));
        assert!(f.can_draw("Pokémon Crystal"));
    }

    #[test]
    fn a_short_name_is_drawn_whole() {
        let mut f = fonts();
        let out = f.render(&Spec::new("Metroid", 14.0, 1.0).wrapped(200.0, 2));
        assert!(!out.clipped, "a name that fits was cut short");
        assert!(out.width > 0 && out.height > 0);
    }

    /// The rule this module exists for: a title too long for its card stops,
    /// and says that it stopped.
    #[test]
    fn a_long_name_is_cut_short_and_says_so() {
        let mut f = fonts();
        let long = "Mortal Kombat II: The Very Long Subtitle Nobody Asked For, Special Edition";
        let out = f.render(&Spec::new(long, 14.0, 1.0).wrapped(150.0, 2));
        assert!(out.clipped, "a name far too long was not cut");
        // And it stayed inside its box.
        assert!(out.width <= 151, "cut to {} points wide, box was 150", out.width);
    }

    /// CJK has no spaces, so "break at spaces" would never break at all and a
    /// Japanese title would run off the side of every card.
    #[test]
    fn japanese_wraps_even_though_it_has_no_spaces() {
        let mut f = fonts();
        let long = "ドラゴンクエストIII そして伝説へ".repeat(3);
        let out = f.render(&Spec::new(&long, 14.0, 1.0).wrapped(150.0, 2));
        assert!(out.width <= 151, "ran off the card: {} points", out.width);
        assert!(out.clipped);
    }

    /// Two lines is two lines. A title that spills onto a third is a card that
    /// overlaps the one below it.
    #[test]
    fn the_line_limit_is_a_limit() {
        let mut f = fonts();
        let long = "Castlevania: Symphony of the Night Deluxe Anniversary Collection";
        for lines in [1u16, 2, 3] {
            let out = f.render(&Spec::new(long, 14.0, 1.0).wrapped(150.0, lines));
            let at_most = (14.0 * 1.3 * lines as f32).ceil() as u32 + 1;
            assert!(out.height <= at_most, "{lines} lines came out {}px tall", out.height);
        }
    }

    /// Scale is part of what a rendered string *is*: the same title at the
    /// same point size is a different set of pixels on a different display.
    #[test]
    fn the_same_text_at_twice_the_scale_is_twice_the_size() {
        let mut f = fonts();
        let one = f.render(&Spec::new("Metroid", 14.0, 1.0));
        let two = f.render(&Spec::new("Metroid", 14.0, 2.0));
        assert!(two.height >= one.height * 2 - 2, "{} vs {}", two.height, one.height);
    }

    /// Empty is not a crash, and neither is a string of nothing but spaces.
    #[test]
    fn nothing_to_draw_draws_nothing() {
        let mut f = fonts();
        for text in ["", "   ", "\n"] {
            let out = f.render(&Spec::new(text, 14.0, 1.0).wrapped(150.0, 2));
            assert!(out.width >= 1 && out.height >= 1, "{text:?} produced no raster at all");
        }
    }

    /// Something actually landed on the page. A rasteriser that silently draws
    /// nothing passes every size assertion above.
    #[test]
    fn the_glyphs_reach_the_page() {
        let mut f = fonts();
        let out = f.render(&Spec::new("Metroid", 24.0, 1.0));
        let ink: usize = out.coverage.iter().filter(|&&v| v > 0).count();
        assert!(ink > 20, "only {ink} pixels of ink for a whole word");
    }
}

#[cfg(test)]
mod scripts {
    use super::*;

    /// Every script a game library actually contains, not just the one that
    /// prompted the work.
    ///
    /// The fallback is per-script and general — nothing here names a language
    /// — so this is a check that the *machine* has the faces, and a list of
    /// what breaks first when it does not.
    const LIBRARY: &[(&str, &str)] = &[
        ("Japanese", "ゼルダの伝説"),
        ("Chinese (simplified)", "塞尔达传说"),
        ("Chinese (traditional)", "薩爾達傳說"),
        ("Korean", "젤다의 전설"),
        ("Cyrillic", "Тетрис"),
        ("Greek", "Ελλάδα"),
        ("Arabic", "لعبة"),
        ("Hebrew", "משחק"),
        ("Thai", "เกม"),
        ("Latin, accented", "Pokémon Crystal"),
    ];

    #[test]
    fn every_script_a_library_holds_finds_a_face() {
        let mut f = Fonts::load().expect("no fonts");
        let mut missing = Vec::new();
        for (name, sample) in LIBRARY {
            if !f.can_draw(sample) {
                missing.push(*name);
            }
        }
        assert!(missing.is_empty(), "no face for: {missing:?}");
    }

    /// Right-to-left is not a font problem, it is a layout one: the shaper has
    /// to reorder. A run that comes back with no glyphs at all means bidi
    /// never ran.
    /// Which face each script is handed to. Printed, not asserted — the
    /// answer depends on what the machine has installed, and the point is to
    /// be able to see it.
    /// The point of all of it: the three writing systems that share code
    /// points are handed to three different faces.
    ///
    /// On a machine with only a pan-CJK fallback installed they will be the
    /// same, and that is the state this exists to make visible rather than to
    /// fail over — so it asserts what it can: that whatever is chosen, the
    /// chooser has an opinion.
    #[test]
    fn each_of_the_shared_scripts_gets_its_own_answer() {
        let mut f = Fonts::load().expect("no fonts");
        let japanese = f.family_for("ゼルダの伝説").map(str::to_owned);
        let simplified = f.family_for("塞尔达传说").map(str::to_owned);
        let traditional = f.family_for("薩爾達傳說").map(str::to_owned);
        let korean = f.family_for("젤다의 전설").map(str::to_owned);
        println!("ja={japanese:?} sc={simplified:?} tc={traditional:?} ko={korean:?}");

        // Latin needs no decision and must not be given one.
        assert_eq!(f.family_for("Metroid"), None);
        assert_eq!(f.family_for("Тетрис"), None);

        // Where the machine has distinct faces, they have to be told apart.
        // This Mac does; the handheld's Noto CJK does too.
        if japanese.is_some() && simplified.is_some() {
            assert_ne!(
                japanese, simplified,
                "Japanese and simplified Chinese were handed to the same face"
            );
        }
        // And every one that is drawn is still drawable.
        for sample in ["ゼルダの伝説", "塞尔达传说", "薩爾達傳說", "젤다의 전설"] {
            assert!(f.can_draw(sample), "{sample} lost its glyphs to the family choice");
        }
    }

    /// Whether the font database reads the individual faces inside a
    /// collection file, or only the first one.
    ///
    /// This decides whether the handheld works at all: Debian's
    /// `fonts-noto-cjk` is a single `NotoSansCJK-Regular.ttc` holding the JP,
    /// KR, SC, TC and HK faces together. If only face zero is seen, every
    /// language gets whichever is first and the Han unification fix silently
    /// does nothing.
    #[test]
    fn faces_inside_a_collection_are_seen_separately() {
        let f = Fonts::load().expect("no fonts");
        let mut collections: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for face in f.system.db().faces() {
            let path = match &face.source {
                cosmic_text::fontdb::Source::File(p) => p.display().to_string(),
                _ => continue,
            };
            if !path.to_lowercase().ends_with(".ttc") {
                continue;
            }
            let name = face.families.first().map(|(n, _)| n.clone()).unwrap_or_default();
            collections.entry(path).or_default().push(name);
        }
        let multi: Vec<_> = collections.iter().filter(|(_, v)| v.len() > 1).collect();
        for (path, names) in multi.iter().take(3) {
            println!("{} -> {} faces: {:?}", path, names.len(), &names[..names.len().min(6)]);
        }
        assert!(
            !multi.is_empty(),
            "no collection file yielded more than one face — Noto Sans CJK on the handheld is \
             one .ttc holding JP, KR, SC and TC, and reading only the first would give every \
             language the same shapes"
        );
    }

    #[test]
    fn which_face_draws_what() {
        let mut f = Fonts::load().expect("no fonts");
        for (name, sample) in LIBRARY {
            let asked = f.family_for(sample).unwrap_or("(any)").to_owned();
            let got = f.face_for(sample);
            println!("{name:<24} {sample:<20} asked {asked:<18} got {got:?}");
        }
    }

    #[test]
    fn right_to_left_still_produces_glyphs() {
        let mut f = Fonts::load().expect("no fonts");
        let out = f.render(&Spec::new("لعبة الفيديو", 16.0, 1.0).wrapped(200.0, 2));
        let ink = out.coverage.iter().filter(|&&v| v > 0).count();
        assert!(ink > 20, "Arabic drew {ink} pixels of ink");
    }

    /// Mixed scripts in one string, which is what a translated title actually
    /// looks like: `Final Fantasy VII ファイナルファンタジーVII`.
    #[test]
    fn one_string_can_span_two_scripts() {
        let mut f = Fonts::load().expect("no fonts");
        assert!(f.can_draw("Final Fantasy VII ファイナルファンタジーVII"));
        assert!(f.can_draw("Street Fighter II 街霸II"));
    }
}
