// Which writing system a title is in, from the title.
//
// Chinese, Japanese and Korean share code points for characters whose correct
// *shapes* differ — 直, 骨, 話, 令 are drawn differently in each, and a reader
// of one sees the other immediately. A font chosen for covering the script
// rather than for the language draws one of them in another's forms. It stays
// perfectly legible and merely looks foreign, which is why nobody reports it.
//
// EmulationStation's answer, on the very handheld this targets, is
// `DroidSansFallbackFull.ttf` — Android's single pan-CJK fallback, built on
// simplified Chinese forms and used for all three. That is the pragmatic
// answer, and it is why Japanese titles look subtly wrong in every one of
// these front ends. `fonts-noto-cjk` is installed there too and has proper
// per-language variants, so the only thing missing is knowing which to ask
// for.
//
// Which the text answers by itself. Kana means Japanese; hangul means Korean;
// Han alone means Chinese, and which Chinese is a question about the
// characters used. No metadata, no region field, no configuration.

/// What to draw a string with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    /// Nothing that needs a *language* decided — Latin, Cyrillic, Greek,
    /// Arabic and the rest, where covering the script is the whole of the
    /// answer. Still names a family, for a different reason: see
    /// [`Script::families`].
    Plain,
    Japanese,
    Korean,
    /// Han characters in the forms used in mainland China and Singapore.
    Simplified,
    /// Han characters in the forms used in Taiwan, Hong Kong and Macau.
    Traditional,
}

/// Characters that exist in only one of the two Chinese writing systems.
///
/// A sample, not a complete table — there are thousands of pairs, and a few
/// dozen common ones settle almost every real title. These are the ones that
/// turn up in game names: 传说 / 傳說, 战 / 戰, 国 / 國, 龙 / 龍.
///
/// Two flat strings rather than pairs, because nothing here needs to know
/// which maps to which. The question is only which set a title draws from.
const SIMPLIFIED_ONLY: &str = "国学传说战汉语门马鸟车东长时会应这边众优义习书买卖见觉关兴举写农军页风飞龙丽点击开区医华叶号电视经济体验单双进过还问题实际动态图标记录网络设备变换级类导览层";
const TRADITIONAL_ONLY: &str = "國學傳說戰漢語門馬鳥車東長時會應這邊眾優義習書買賣見覺關興舉寫農軍頁風飛龍麗點擊開區醫華葉號電視經濟體驗單雙進過還問題實際動態圖標記錄網絡設備變換級類導覽層";

/// Which writing system this string wants to be drawn in.
///
/// Deliberately a guess, and the guesses are ordered by how certain they are.
/// Kana and hangul are proof. Han alone is not, so the simplified and
/// traditional forms present are counted and the larger set wins — with a tie
/// going to simplified, which is the more common of the two and what every
/// pan-CJK fallback font already draws.
pub fn of(text: &str) -> Script {
    let mut han = false;
    let (mut simplified, mut traditional) = (0usize, 0usize);

    for c in text.chars() {
        // Kana. Nothing else uses it, so one character settles the question —
        // except the two marks below, which are borrowed by everybody.
        if matches!(c, '\u{3040}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}')
            && !matches!(c, '\u{30FB}' | '\u{30FC}')
        {
            return Script::Japanese;
        }
        // Hangul, in all of its blocks.
        if matches!(
            c,
            '\u{1100}'..='\u{11FF}'
                | '\u{3130}'..='\u{318F}'
                | '\u{A960}'..='\u{A97F}'
                | '\u{AC00}'..='\u{D7AF}'
        ) {
            return Script::Korean;
        }
        if is_han(c) {
            han = true;
            if SIMPLIFIED_ONLY.contains(c) {
                simplified += 1;
            }
            if TRADITIONAL_ONLY.contains(c) {
                traditional += 1;
            }
        }
    }

    if !han {
        return Script::Plain;
    }
    if traditional > simplified { Script::Traditional } else { Script::Simplified }
}

fn is_han(c: char) -> bool {
    matches!(
        c,
        '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{20000}'..='\u{2A6DF}'
    )
}

impl Script {
    /// The families to ask for, best first.
    ///
    /// Names rather than a generic family, because "sans-serif" is exactly the
    /// request that loses this argument. Several per script so one list serves
    /// the handheld's Noto and a desktop's own faces; whichever is installed
    /// first wins, and none installed means fall back to the generic and
    /// accept whatever covers the glyphs.
    ///
    /// Even `Plain` names one, and not for coverage — for sameness. One front
    /// end that looks different on every machine because each supplied its own
    /// idea of "sans-serif" is not one front end. Noto Sans is shipped with
    /// the app (see `assets/fonts/MANIFEST.tsv`), so it is there to ask for;
    /// where it somehow is not, the generic still answers.
    pub fn families(self) -> &'static [&'static str] {
        match self {
            Script::Plain => &["Noto Sans"],
            Script::Japanese => &["Noto Sans CJK JP", "Noto Sans JP", "Hiragino Sans", "Yu Gothic"],
            Script::Korean => {
                &["Noto Sans CJK KR", "Noto Sans KR", "Apple SD Gothic Neo", "Malgun Gothic"]
            }
            Script::Simplified => {
                &["Noto Sans CJK SC", "Noto Sans SC", "PingFang SC", "Microsoft YaHei"]
            }
            Script::Traditional => {
                &["Noto Sans CJK TC", "Noto Sans TC", "PingFang TC", "Microsoft JhengHei"]
            }
        }
    }

    /// The BCP 47 tag, for a front end that can say it outright — a webview
    /// setting `lang` on the element fixes exactly this problem in exactly
    /// this way, with no font names involved.
    pub fn language_tag(self) -> Option<&'static str> {
        match self {
            Script::Plain => None,
            Script::Japanese => Some("ja"),
            Script::Korean => Some("ko"),
            Script::Simplified => Some("zh-Hans"),
            Script::Traditional => Some("zh-Hant"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kana is proof. One character of it settles the answer however much Han
    /// is in the rest of the title.
    #[test]
    fn kana_means_japanese() {
        assert_eq!(of("ゼルダの伝説"), Script::Japanese);
        assert_eq!(of("ドラゴンクエストIII そして伝説へ"), Script::Japanese);
        assert_eq!(of("スーパーマリオブラザーズ3"), Script::Japanese);
        assert_eq!(of("鬼武者の伝説"), Script::Japanese);
    }

    #[test]
    fn hangul_means_korean() {
        assert_eq!(of("젤다의 전설"), Script::Korean);
        assert_eq!(of("메탈슬러그"), Script::Korean);
    }

    /// Han with no kana and no hangul is Chinese. Which one is the part worth
    /// getting right.
    #[test]
    fn han_alone_is_chinese_and_we_say_which() {
        assert_eq!(of("塞尔达传说"), Script::Simplified);
        assert_eq!(of("薩爾達傳說"), Script::Traditional);
        assert_eq!(of("最终幻想"), Script::Simplified);
        assert_eq!(of("三國志"), Script::Traditional);
        assert_eq!(of("仙剑奇侠传"), Script::Simplified);
    }

    /// A title made only of characters both systems write identically. There
    /// is no right answer, so it picks the commoner one rather than nothing.
    #[test]
    fn a_title_that_does_not_say_falls_to_simplified() {
        assert_eq!(of("音速小子"), Script::Simplified);
    }

    #[test]
    fn everything_else_needs_no_decision() {
        assert_eq!(of("Metroid"), Script::Plain);
        assert_eq!(of("Pokémon Crystal"), Script::Plain);
        assert_eq!(of("Тетрис"), Script::Plain);
        assert_eq!(of("لعبة"), Script::Plain);
        assert_eq!(of(""), Script::Plain);
    }

    /// The common real case: an English name beside its Japanese one.
    #[test]
    fn a_title_in_two_scripts_takes_the_one_that_needs_deciding() {
        assert_eq!(of("Final Fantasy VII ファイナルファンタジーVII"), Script::Japanese);
        assert_eq!(of("Street Fighter II 街霸II"), Script::Simplified);
        assert_eq!(of("Street Fighter II 快打旋風II"), Script::Traditional);
    }

    /// The long vowel mark and the middle dot live in the katakana block but
    /// turn up in Chinese and Korean titles too, so neither is proof.
    #[test]
    fn a_borrowed_katakana_mark_does_not_settle_it() {
        assert_eq!(of("三國志・戰略版"), Script::Traditional);
        assert_eq!(of("塞尔达・传说"), Script::Simplified);
    }

    #[test]
    fn every_script_names_a_font_and_a_language() {
        for script in [Script::Japanese, Script::Korean, Script::Simplified, Script::Traditional] {
            assert!(!script.families().is_empty(), "{script:?} asks for no font");
            assert!(script.language_tag().is_some(), "{script:?} has no language tag");
        }
        // Plain names a family too, for sameness across machines rather than
        // for coverage — but it is not a *language*, and must not claim to be.
        assert_eq!(Script::Plain.families(), ["Noto Sans"]);
        assert_eq!(Script::Plain.language_tag(), None);
    }
}
