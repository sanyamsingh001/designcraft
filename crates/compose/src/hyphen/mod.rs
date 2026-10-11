//! Language-specific hyphenation.
//!
//! Spanish uses Javier Bezos's `hyph-es.tex` 5.0 patterns through `hypher` (MIT; see NOTICE).
//! Russian uses Alexander I. Lebedev's published Liang patterns and explicit exceptions
//! (LPPL-1.2-or-later; see `README.md` here). Unsupported languages have no automatic breaks.
//! The existing convenience functions default to US English; composition selects the resolved
//! character language explicitly. No script detection changes the document's language.
//!
//! Lookup order for each English word:
//! 1. the **dictionary** ([`Dictionary::en_us`]) — the public-domain Moby Hyphenator II word list
//!    by Grady Ward (~160k words), stored compactly in `assets/hyphenation/en-us.dic`; simple
//!    inflections (`'s`, `-s`, `-es`, `-ed`, `-d`) inherit the breaks of their stem;
//! 2. **Liang patterns** ([`Patterns::en_us`]) for words not in the dictionary — our own pattern
//!    set, trained from the same list by [`patgen`] (`examples/hyphgen.rs` regenerates both files);
//! 3. a small syllable **heuristic** for ASCII words outside the pattern alphabet.
//!
//! Then the paragraph's limits ([`Limits`]) are applied.

pub mod dict;
pub mod patgen;
pub mod patterns;

use std::sync::OnceLock;
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};
use unicode_segmentation::UnicodeSegmentation;

pub use dict::Dictionary;
pub use patterns::Patterns;

static EN_US_DIC: &[u8] = include_bytes!("../../../../assets/hyphenation/en-us.dic");
static EN_US_PAT: &str = include_str!("../../../../assets/hyphenation/en-us.pat");
static RU_TEX: &str = include_str!("hyph-ru.tex");

/// Languages with bundled hyphenation data. An unsupported language has no automatic breaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    English,
    Russian,
    Patterns(hypher::Lang),
}

impl Lang {
    /// Select data from a resolved BCP 47 language tag.
    pub fn from_tag(tag: &str) -> Option<Self> {
        match designcraft_doc::language_subtag(tag) {
            t if t.eq_ignore_ascii_case("en") => Some(Self::English),
            t if t.eq_ignore_ascii_case("ru") => Some(Self::Russian),
            sub => {
                let b = sub.as_bytes();
                if b.len() != 2 {
                    return None;
                }
                hypher::Lang::from_iso([b[0].to_ascii_lowercase(), b[1].to_ascii_lowercase()]).map(Self::Patterns)
            }
        }
    }

    /// Select data from a document language name or locale code (including IDML names).
    pub fn for_language(name: &str) -> Option<Self> {
        designcraft_doc::language_tag(name).and_then(Self::from_tag)
    }
}

// The bundled TeX source is unchanged. Only its two data blocks are interpreted; TeX is never
// executed. Keeping the original source preserves its copyright, license and provenance.
fn russian_block(command: &str) -> Option<&'static str> {
    RU_TEX.split_once(command)?.1.split_once('}').map(|(block, _)| block)
}

impl Dictionary {
    /// The Russian pattern author's explicit exceptions (parsed on first use).
    pub fn ru() -> &'static Dictionary {
        static D: OnceLock<Dictionary> = OnceLock::new();
        D.get_or_init(|| {
            let Some(block) = russian_block("\\hyphenation{") else {
                log::error!("Russian hyphenation: missing exception block");
                return Dictionary::default();
            };
            Dictionary::from_entries(
                block.lines().flat_map(|line| line.split('%').next().unwrap_or("").split_whitespace()).map(dict::parse_hyphenated).collect(),
            )
        })
    }

    /// The bundled US English dictionary (decoded on first use).
    pub fn en_us() -> &'static Dictionary {
        static D: OnceLock<Dictionary> = OnceLock::new();
        D.get_or_init(|| {
            if EN_US_DIC.is_empty() {
                return Dictionary::default();
            }
            Dictionary::from_bytes(EN_US_DIC).unwrap_or_else(|e| {
                log::error!("{e}");
                Dictionary::default()
            })
        })
    }
}

impl Patterns {
    /// The bundled Russian patterns (parsed on first use).
    pub fn ru() -> &'static Patterns {
        static P: OnceLock<Patterns> = OnceLock::new();
        P.get_or_init(|| {
            let Some(block) = russian_block("\\patterns{") else {
                log::error!("Russian hyphenation: missing pattern block");
                return Patterns::default();
            };
            Patterns::parse(block)
        })
    }

    /// The bundled US English patterns (parsed on first use).
    pub fn en_us() -> &'static Patterns {
        static P: OnceLock<Patterns> = OnceLock::new();
        P.get_or_init(|| Patterns::parse(EN_US_PAT))
    }
}

/// Hyphenation limits from the paragraph's Hyphenation settings.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub min_word: usize,
    pub after_first: usize,
    pub before_last: usize,
    pub capitalized: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { min_word: 5, after_first: 2, before_last: 2, capitalized: true }
    }
}

/// Where a word's break points came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Dictionary,
    Patterns,
    Heuristic,
    None,
}

fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '’' | 'ʼ')
}

fn is_word_char(c: char) -> bool {
    c.is_alphabetic() || is_apostrophe(c)
}

// Unsupported/oversized letter graphemes stay in the word and reject the whole run;
// treating them as punctuation would incorrectly hyphenate a Russian suffix in isolation.
const INVALID_RUSSIAN_GRAPHEME: char = '\u{fffd}';
const MAX_RUSSIAN_GRAPHEME_CHARS: usize = 32;

fn is_run_char(c: char, language: Lang) -> bool {
    is_word_char(c) || (language == Lang::Russian && c == INVALID_RUSSIAN_GRAPHEME)
}

/// One lookup character per source grapheme, with boundaries in ORIGINAL scalar indices.
/// The bounded per-grapheme NFC operation never normalizes a document or a glyph stream.
struct RussianWord {
    chars: Vec<char>,
    source_boundaries: Vec<usize>,
}

impl RussianWord {
    fn new(word: &str) -> Self {
        let mut chars = Vec::new();
        let mut source_boundaries = vec![0];
        let mut source_end = 0usize;
        for grapheme in word.graphemes(true) {
            let source_len = grapheme.chars().count();
            let Some(first) = grapheme.chars().next() else { continue };
            let c = if source_len == 1 && matches!(first, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё') {
                first
            } else if is_word_char(first) || is_combining_mark(first) {
                if source_len > MAX_RUSSIAN_GRAPHEME_CHARS {
                    INVALID_RUSSIAN_GRAPHEME
                } else {
                    // NFD also exposes canonically equivalent stress forms. Strip only acute
                    // and grave stress, then NFC composes е + diaeresis / и + breve. Keeping
                    // case lets paragraph capitalization limits operate on the lookup letters.
                    let mut normalized = grapheme.nfd().filter(|&c| !matches!(c, '\u{0300}' | '\u{0301}')).nfc();
                    match (normalized.next(), normalized.next()) {
                        (Some(c), None) if !is_combining_mark(c) => c,
                        _ => INVALID_RUSSIAN_GRAPHEME,
                    }
                }
            } else {
                first
            };
            chars.push(c);
            source_end = source_end.saturating_add(source_len);
            source_boundaries.push(source_end);
        }
        Self { chars, source_boundaries }
    }

    fn source_points(&self, points: Vec<usize>) -> Vec<usize> {
        points.into_iter().filter_map(|p| self.source_boundaries.get(p).copied()).collect()
    }
}

/// Unrestricted break points of one lowercase core (letters and `'` only, no leading/trailing `'`).
fn core_breaks(core: &[char], language: Lang) -> (Vec<usize>, Source) {
    let n = core.len();
    if n < 2 {
        return (vec![], Source::None);
    }
    let dict = match language {
        Lang::Patterns(l) => {
            if core.contains(&'\'') {
                return (vec![], Source::None);
            }
            let s: String = core.iter().collect();
            let mut out = Vec::new();
            let mut at = 0;
            let syl: Vec<&str> = hypher::hyphenate_bounded(&s, l, 1, 1).collect();
            for part in &syl[..syl.len().saturating_sub(1)] {
                at += part.chars().count();
                out.push(at);
            }
            return (out, Source::Patterns);
        }
        Lang::English => Dictionary::en_us(),
        Lang::Russian => {
            // Russian data is for Russian letters only, including ё. Reject letters outside
            // that alphabet rather than applying Russian rules to a mixed word.
            if !core.iter().all(|&c| matches!(c, 'а'..='я' | 'ё')) {
                return (vec![], Source::None);
            }
            Dictionary::ru()
        }
    };
    let s: String = core.iter().collect();
    let from_mask = |m: u64, len: usize| (1..len.min(64)).filter(|&i| m & (1 << i) != 0).collect::<Vec<_>>();
    if let Some(m) = dict.get(&s) {
        return (from_mask(m, n), Source::Dictionary);
    }
    // Inflections inherit the stem's breaks (the suffix adds no break of its own).
    for &suf in if language == Lang::English { &["'s", "s", "es", "ed", "d"][..] } else { &[] } {
        if let Some(stem) = s.strip_suffix(suf)
            && stem.chars().count() >= 2
            && let Some(m) = dict.get(stem)
        {
            return (from_mask(m, stem.chars().count()), Source::Dictionary);
        }
    }
    let pats = match language {
        Lang::English | Lang::Patterns(_) => Patterns::en_us(),
        Lang::Russian => Patterns::ru(),
    };
    if !pats.is_empty()
        && let Some(p) = pats.points(core)
    {
        return (p, Source::Patterns);
    }
    if language == Lang::English && core.iter().all(char::is_ascii_alphabetic) {
        (heuristic_points(core), Source::Heuristic)
    } else {
        (vec![], Source::None)
    }
}

/// Unrestricted break points of `word` (char indices; a hyphen goes before that char), handling
/// case, apostrophes (`don't`, `’s`), surrounding punctuation and compounds (each letter run is
/// hyphenated separately). No break is ever placed next to an apostrophe or non-letter.
pub fn word_breaks(word: &str) -> (Vec<usize>, Source) {
    word_breaks_in(word, Lang::English)
}

/// Unrestricted break points using the selected language's data. See [`word_breaks`] for the
/// indexing and punctuation rules. Russian composes decomposed letters and ignores acute/grave
/// stress for lookup; returned points remain original source character indices at grapheme
/// boundaries. Russian never falls back to English or a syllable heuristic.
pub fn word_breaks_in(word: &str, language: Lang) -> (Vec<usize>, Source) {
    if language == Lang::Russian {
        let normalized = RussianWord::new(word);
        let (points, source) = word_breaks_from_chars(&normalized.chars, language);
        return (normalized.source_points(points), source);
    }
    word_breaks_from_chars(&word.chars().collect::<Vec<_>>(), language)
}

fn word_breaks_from_chars(chars: &[char], language: Lang) -> (Vec<usize>, Source) {
    let mut out = Vec::new();
    let mut src = Source::None;
    let mut i = 0;
    while i < chars.len() {
        if !is_run_char(chars[i], language) {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < chars.len() && is_run_char(chars[j], language) {
            j += 1;
        }
        let (mut a, mut b) = (i, j);
        while a < b && is_apostrophe(chars[a]) {
            a += 1;
        }
        while b > a && is_apostrophe(chars[b - 1]) {
            b -= 1;
        }
        let core: Vec<char> = chars[a..b].iter().map(|&c| if is_apostrophe(c) { '\'' } else { c.to_lowercase().next().unwrap_or(c) }).collect();
        let (pts, s) = core_breaks(&core, language);
        if src == Source::None {
            src = s;
        }
        out.extend(pts.into_iter().filter(|&p| p > 0 && p < core.len() && core[p - 1] != '\'' && core[p] != '\'').map(|p| a + p));
        i = j;
    }
    (out, src)
}

/// Allowed hyphenation points in `word` as char indices (a hyphen goes before that char), with
/// the paragraph limits applied to each letter run.
pub fn hyphen_points(word: &str, lim: &Limits) -> Vec<usize> {
    hyphen_points_in(word, lim, Lang::English)
}

/// Allowed points from the selected language's data, with paragraph limits applied. Russian
/// additionally requires at least two letters on each side, as prescribed by its pattern set.
pub fn hyphen_points_in(word: &str, lim: &Limits, language: Lang) -> Vec<usize> {
    if language == Lang::Russian {
        let normalized = RussianWord::new(word);
        return normalized.source_points(limited_points(&normalized.chars, lim, language));
    }
    limited_points(&word.chars().collect::<Vec<_>>(), lim, language)
}

fn limited_points(chars: &[char], lim: &Limits, language: Lang) -> Vec<usize> {
    let (pts, _) = word_breaks_from_chars(chars, language);
    if pts.is_empty() {
        return pts;
    }
    let min_fragment = if language == Lang::Russian { 2 } else { 1 };
    let min_before = lim.after_first.max(min_fragment);
    let min_after = lim.before_last.max(min_fragment);
    // Run bounds of each point's letter run (trimmed of apostrophes).
    let run_of = |p: usize| {
        let mut a = p;
        while a > 0 && is_run_char(chars[a - 1], language) {
            a -= 1;
        }
        let mut b = p;
        while b < chars.len() && is_run_char(chars[b], language) {
            b += 1;
        }
        while a < b && is_apostrophe(chars[a]) {
            a += 1;
        }
        while b > a && is_apostrophe(chars[b - 1]) {
            b -= 1;
        }
        (a, b)
    };
    pts.into_iter()
        .filter(|&p| {
            let (a, b) = run_of(p);
            let run = &chars[a..b];
            let letters = run.iter().filter(|c| c.is_alphabetic()).count();
            if letters < lim.min_word.max(2) {
                return false;
            }
            if letters > 1 && run.iter().filter(|c| c.is_alphabetic()).all(|c| c.is_uppercase()) {
                return false;
            }
            if !lim.capitalized && run[0].is_uppercase() {
                return false;
            }
            let before = chars[a..p].iter().filter(|c| c.is_alphabetic()).count();
            let after = chars[p..b].iter().filter(|c| c.is_alphabetic()).count();
            before >= min_before && after >= min_after
        })
        .collect()
}

/// `word` with `-` at every hyphenation point (tests and diagnostics).
pub fn hyphenate_word(word: &str, lim: &Limits) -> String {
    hyphenate_word_in(word, lim, Lang::English)
}

/// Diagnostic form of [`hyphen_points_in`], inserting a hyphen at every allowed point.
pub fn hyphenate_word_in(word: &str, lim: &Limits, language: Lang) -> String {
    let pts = hyphen_points_in(word, lim, language);
    let mut s = String::with_capacity(word.len() + pts.len());
    for (i, ch) in word.chars().enumerate() {
        if pts.contains(&i) {
            s.push('-');
        }
        s.push(ch);
    }
    s
}

// ---------- heuristic fallback (words outside the pattern alphabet) ----------

fn is_vowel(c: char) -> bool {
    matches!(c.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u' | 'y')
}

fn onset_pair(a: char, b: char) -> bool {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    matches!(
        (a, b),
        ('c' | 's' | 't' | 'p' | 'w' | 'g', 'h')
            | ('b' | 'c' | 'd' | 'f' | 'g' | 'k' | 'p' | 't', 'r')
            | ('b' | 'c' | 'f' | 'g' | 'k' | 'p' | 's', 'l')
            | ('q', 'u')
            | ('s', 't' | 'p' | 'c' | 'k')
    )
}

/// Syllable heuristic: V-CV ("ty-po"), VC-CV ("hap-pen"), keeping onset clusters ("gra-phy").
fn heuristic_points(c: &[char]) -> Vec<usize> {
    let n = c.len();
    if n < 4 || !c.iter().all(|c| c.is_alphabetic()) {
        return vec![];
    }
    let v: Vec<bool> = c.iter().map(|&x| is_vowel(x)).collect();
    let mut out = vec![];
    for i in 1..n {
        let ok = if v[i - 1] && !v[i] {
            (i + 1 < n && v[i + 1]) || (i + 2 < n && onset_pair(c[i], c[i + 1]) && v[i + 2])
        } else if !v[i - 1] && !v[i] {
            i >= 2 && v[i - 2] && i + 1 < n && v[i + 1] && !onset_pair(c[i - 1], c[i]) && c[i - 1] != c[i] || (c[i - 1] == c[i] && i >= 2 && v[i - 2])
        } else {
            false
        };
        // Both parts need a sounded vowel: a trailing silent `e`, `ed` or `es` doesn't count.
        let right: String = c[i..].iter().collect();
        let core = right.strip_suffix("ed").or_else(|| right.strip_suffix("es")).or_else(|| right.strip_suffix('e')).unwrap_or(&right);
        if ok && core.chars().any(is_vowel) && core.len() > 1 && v[..i].iter().any(|x| *x) {
            out.push(i);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(w: &str) -> String {
        hyphenate_word(w, &Limits::default())
    }

    #[test]
    fn bundled_data_loads() {
        assert!(Dictionary::en_us().len() > 150_000, "{}", Dictionary::en_us().len());
        assert!(Patterns::en_us().len() > 1000, "{}", Patterns::en_us().len());
    }

    #[test]
    fn dictionary_words() {
        assert_eq!(h("happen"), "hap-pen");
        assert_eq!(h("typography"), "ty-pog-ra-phy");
        assert_eq!(h("composition"), "com-po-si-tion");
        assert_eq!(h("hyphenation"), "hy-phen-a-tion");
        assert_eq!(h("international"), "in-ter-na-tion-al");
        assert_eq!(h("cat"), "cat");
        assert!(!h("changed").contains('-'));
        assert_eq!(h("NASA"), "NASA");
        assert_eq!(word_breaks("publication").1, Source::Dictionary);
    }

    #[test]
    fn case_apostrophes_and_punctuation() {
        assert_eq!(h("Typography"), "Ty-pog-ra-phy");
        assert_eq!(h("TYPOGRAPHY"), "TYPOGRAPHY");
        assert_eq!(h("typography,"), "ty-pog-ra-phy,");
        assert_eq!(h("“typography”"), "“ty-pog-ra-phy”");
        assert_eq!(h("typography's"), "ty-pog-ra-phy's");
        assert_eq!(h("typography’s"), "ty-pog-ra-phy’s");
        assert_eq!(h("'typography'"), "'ty-pog-ra-phy'");
        // Compounds: each part separately, never next to the hyphen.
        assert_eq!(h("typography-composition"), "ty-pog-ra-phy-com-po-si-tion");
    }

    #[test]
    fn inflections_inherit_the_stem() {
        assert_eq!(h("ripples"), "rip-ples");
        assert_eq!(h("gradients"), "gra-di-ents");
        assert_eq!(h("restrained"), "re-strained");
    }

    #[test]
    fn patterns_cover_unknown_words() {
        // Not in the word list: the trained patterns decide.
        let (pts, src) = word_breaks("hyperlocalization");
        assert_eq!(src, Source::Patterns);
        assert!(pts.len() >= 4, "{}", h("hyperlocalization"));
        let (_, src) = word_breaks("zzyzx");
        assert_ne!(src, Source::Dictionary);
    }

    #[test]
    fn limits_are_respected() {
        let lim = Limits { min_word: 5, after_first: 3, before_last: 3, capitalized: false };
        for w in ["typography", "hyphenation", "justification", "Paragraph", "beautiful"] {
            for p in hyphen_points(w, &lim) {
                assert!(p >= 3 && p <= w.chars().count() - 3, "{w} {p}");
            }
        }
        assert!(hyphen_points("Paragraph", &lim).is_empty());
        let lim = Limits { min_word: 12, ..Limits::default() };
        assert!(hyphen_points("typography", &lim).is_empty());
    }

    #[test]
    fn spanish_uses_the_bezos_patterns() {
        let es = Lang::for_language("Spanish").unwrap();
        assert_eq!(es, Lang::for_language("es_ES").unwrap());
        let h = |w: &str| {
            let pts = hyphen_points_in(w, &Limits::default(), es);
            w.chars().enumerate().map(|(i, c)| if pts.contains(&i) { format!("-{c}") } else { c.to_string() }).collect::<String>()
        };
        assert_eq!(h("transición"), "tran-si-ción");
        assert_eq!(h("pingüino"), "pin-güino");
        assert_eq!(h("Construcción"), "Cons-truc-ción");
        assert_eq!(h("desesperación"), "de-ses-pe-ra-ción");
        assert_eq!(h("teatro"), "tea-tro", "a vowel run stays whole");
        assert_eq!(h("calle"), "ca-lle", "ll stays whole");
        assert!(Lang::for_language("French").is_none(), "only languages built in");
    }

    #[test]
    fn language_dispatch_uses_names_and_locales() {
        for name in ["English: USA", "English: UK", "$ID/English: Australian", "en-US", "EN_gb"] {
            assert_eq!(Lang::for_language(name), Some(Lang::English), "{name}");
        }
        for name in ["Russian", "russian", "$ID/Russian", "ru", "ru-RU", "RU_ru"] {
            assert_eq!(Lang::for_language(name), Some(Lang::Russian), "{name}");
        }
        let spanish = Lang::for_language("Spanish");
        assert!(matches!(spanish, Some(Lang::Patterns(_))));
        for tag in ["es", "es-ES", "ES-ES"] {
            assert_eq!(Lang::from_tag(tag), spanish, "{tag}");
        }
        for name in ["French", "Ukrainian", "Bulgarian", "uk-UA", "[No Language]", "Englishness", "", "unknown"] {
            assert_eq!(Lang::for_language(name), None, "{name}");
        }
    }

    #[test]
    fn russian_data_loads_without_dropping_patterns() {
        let block = russian_block("\\patterns{").unwrap();
        let mut parsed = Patterns::default();
        let mut count = 0;
        for line in block.lines() {
            for token in line.split('%').next().unwrap_or("").split_whitespace() {
                assert!(parsed.insert(token), "unsupported Russian pattern: {token}");
                count += 1;
            }
        }
        assert_eq!(count, 7021);
        assert_eq!(parsed.len(), count);
        assert_eq!(Patterns::ru().len(), count);
        assert_eq!(Dictionary::ru().len(), 184);
    }

    #[test]
    fn russian_patterns_and_exceptions() {
        for (word, expected) in [
            ("типография", "ти-по-гра-фия"),
            ("перенос", "пе-ре-нос"),
            ("молоко", "мо-ло-ко"),
            ("берёза", "бе-рё-за"),
            ("русский", "рус-ский"),
            ("подъезд", "подъ-езд"),
            ("объединение", "объ-еди-не-ние"),
            ("майка", "май-ка"),
            ("семья", "се-мья"),
            ("письмо", "пись-мо"),
            ("международный", "меж-ду-на-род-ный"),
            ("однажды", "од-на-жды"),
            ("бизнесмен", "биз-нес-мен"),
            ("помните", "помни-те"),
            ("умру", "умру"),
        ] {
            assert_eq!(hyphenate_word_in(word, &Limits::default(), Lang::Russian), expected, "{word}");
        }
        assert_eq!(word_breaks_in("типография", Lang::Russian), (vec![2, 4, 7, 9], Source::Patterns));
        assert_eq!(word_breaks_in("помните", Lang::Russian), (vec![5], Source::Dictionary));
        assert_eq!(word_breaks_in("умру", Lang::Russian), (vec![], Source::Dictionary));
    }

    #[test]
    fn russian_case_punctuation_and_limits_use_character_indices() {
        let lim = Limits::default();
        assert_eq!(hyphenate_word_in("«Берёза»,", &lim, Lang::Russian), "«Бе-рё-за»,");
        assert_eq!(hyphenate_word_in("майка-письмо", &lim, Lang::Russian), "май-ка-пись-мо");
        assert_eq!(hyphenate_word_in("ТИПОГРАФИЯ", &lim, Lang::Russian), "ТИПОГРАФИЯ");
        assert!(hyphen_points_in("Типография", &Limits { capitalized: false, ..lim }, Lang::Russian).is_empty());
        assert!(hyphen_points_in("типография", &Limits { min_word: 11, ..lim }, Lang::Russian).is_empty());
        assert_eq!(hyphen_points_in("типография", &Limits { after_first: 3, before_last: 4, ..lim }, Lang::Russian), vec![4]);
        let relaxed = Limits { min_word: 2, after_first: 1, before_last: 1, ..lim };
        assert_eq!(hyphenate_word_in("ёлочка", &relaxed, Lang::Russian), "ёлоч-ка");
        assert_eq!(hyphenate_word_in("язык", &relaxed, Lang::Russian), "язык");
    }

    #[test]
    fn russian_decomposed_letters_return_source_character_indices() {
        for (word, points, display) in [
            ("бере\u{0308}за", vec![2, 5], "бе-ре\u{0308}-за"),
            ("маи\u{0306}ка", vec![4], "маи\u{0306}-ка"),
            ("е\u{0308}лочка", vec![5], "е\u{0308}лоч-ка"),
            ("«Бере\u{0308}за»,", vec![3, 6], "«Бе-ре\u{0308}-за»,"),
            ("бере\u{0308}за-маи\u{0306}ка", vec![2, 5, 12], "бе-ре\u{0308}-за-маи\u{0306}-ка"),
        ] {
            assert_eq!(hyphen_points_in(word, &Limits::default(), Lang::Russian), points, "{word}");
            assert_eq!(hyphenate_word_in(word, &Limits::default(), Lang::Russian), display);
            let boundaries: Vec<_> = word.grapheme_indices(true).map(|(byte, _)| word[..byte].chars().count()).collect();
            assert!(points.iter().all(|p| boundaries.contains(p)), "{word}: all source points must end graphemes");
        }
        assert_eq!(word_breaks_in("бере\u{0308}за", Lang::Russian), (vec![2, 5], Source::Patterns));
    }

    #[test]
    fn russian_stress_is_transparent_but_positions_include_every_source_mark() {
        let word = "типография";
        for stress in ['\u{0300}', '\u{0301}', '\u{0341}'] {
            for marked in 0..word.chars().count() {
                let mut source = String::new();
                for (i, c) in word.chars().enumerate() {
                    source.push(c);
                    if i == marked {
                        source.push(stress);
                    }
                }
                let map = |p: usize| p + usize::from(marked < p);
                assert_eq!(word_breaks_in(&source, Lang::Russian), (vec![map(2), map(4), map(7), map(9)], Source::Patterns), "{source}");
                assert_eq!(hyphen_points_in(&source, &Limits::default(), Lang::Russian), vec![map(2), map(4), map(7)], "{source}");
            }
        }
        for source in ["бере\u{0308}\u{0301}за", "бере\u{0301}\u{0308}за"] {
            assert_eq!(hyphen_points_in(source, &Limits::default(), Lang::Russian), vec![2, 6]);
        }
    }

    #[test]
    fn russian_unicode_limits_count_lookup_letters_and_preserve_case() {
        let word = "типо\u{0301}графия";
        let lim = Limits { after_first: 3, before_last: 3, ..Limits::default() };
        assert_eq!(hyphen_points_in(word, &lim, Lang::Russian), vec![5, 8]);
        assert!(hyphen_points_in(word, &Limits { min_word: 11, ..lim }, Lang::Russian).is_empty());
        assert!(hyphen_points_in("МАИ\u{0306}КА", &Limits::default(), Lang::Russian).is_empty());
        assert!(hyphen_points_in("Бере\u{0308}за", &Limits { capitalized: false, ..Limits::default() }, Lang::Russian).is_empty());
        assert!(hyphen_points_in("язы\u{0301}к", &Limits { min_word: 2, after_first: 1, before_last: 1, ..lim }, Lang::Russian).is_empty());
    }

    #[test]
    fn russian_stressed_upstream_exceptions_keep_precedence() {
        for word in ["помни\u{0301}те", "по\u{0301}мните"] {
            assert_eq!(word_breaks_in(word, Lang::Russian), (vec![6], Source::Dictionary));
            assert_eq!(hyphen_points_in(word, &Limits::default(), Lang::Russian), vec![6]);
        }
        assert_eq!(word_breaks_in("у\u{0301}мру", Lang::Russian), (vec![], Source::Dictionary));
    }

    #[test]
    fn unsupported_and_oversized_graphemes_reject_the_entire_russian_run() {
        for word in ["ти\u{0327}пография", "\u{0301}типография", "типо\u{200d}графия", "молo\u{0301}ко", "украї\u{0301}нський"]
        {
            assert_eq!(word_breaks_in(word, Lang::Russian), (vec![], Source::None), "{word}");
        }
        let word = format!("ти{}пография", "\u{0301}".repeat(MAX_RUSSIAN_GRAPHEME_CHARS));
        assert_eq!(word_breaks_in(&word, Lang::Russian), (vec![], Source::None));
        assert!(hyphen_points_in(&word, &Limits::default(), Lang::Russian).is_empty());
        for word in ["бере\u{0308}за", "маи\u{0306}ка", "типо\u{0301}графия"] {
            assert!(hyphen_points_in(word, &Limits::default(), Lang::English).is_empty(), "no implicit language detection");
        }
    }

    #[test]
    fn language_data_never_crosses_alphabets() {
        for word in ["typography", "молoко", "український", "српскијезик"] {
            assert_eq!(word_breaks_in(word, Lang::Russian), (vec![], Source::None), "{word}");
        }
        for word in ["типография", "молoко", "Ελληνικά"] {
            assert_eq!(word_breaks(word), (vec![], Source::None), "{word}");
        }
    }

    #[test]
    fn heuristic_fallback() {
        let c: Vec<char> = "happen".chars().collect();
        assert_eq!(heuristic_points(&c), vec![3]);
    }
}
