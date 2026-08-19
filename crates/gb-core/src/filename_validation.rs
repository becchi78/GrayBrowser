//! Machine-dependent character detection for filenames. Pure logic: no
//! filesystem access, no `#[cfg(windows)]`.

/// Unicode ranges (inclusive) treated as machine-dependent characters.
///
/// `U+3251..=U+32FF` (circled-kanji IBM extension) is a subset of
/// `U+3200..=U+33FF` (Enclosed CJK Letters and Months / CJK Compatibility) and
/// is therefore folded into that single range rather than listed separately.
const MACHINE_DEPENDENT_RANGES: &[(char, char)] = &[
    ('\u{2460}', '\u{24FF}'), // 丸囲み数字 (Enclosed Alphanumerics)
    ('\u{2160}', '\u{2188}'), // ローマ数字 (Number Forms)
    ('\u{3200}', '\u{33FF}'), // 単位記号・元号合字・丸囲み漢字拡張 (Enclosed CJK Letters and Months / CJK Compatibility)
    ('\u{2660}', '\u{2667}'), // トランプ柄記号
    ('\u{E000}', '\u{F8FF}'), // 外字領域 (Private Use Area) -- unconditionally machine-dependent
];

fn is_machine_dependent_char(c: char) -> bool {
    MACHINE_DEPENDENT_RANGES
        .iter()
        .any(|&(start, end)| c >= start && c <= end)
}

/// Returns the first machine-dependent character found in `name`, or `None`
/// if the filename contains none. Scans the whole string, including the
/// extension.
pub fn is_machine_dependent_filename(name: &str) -> Option<char> {
    name.chars().find(|&c| is_machine_dependent_char(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- 丸囲み数字 (U+2460..=U+24FF) --
    #[test]
    fn enclosed_alphanumerics_just_before_range_is_not_flagged() {
        assert_eq!(is_machine_dependent_filename("\u{245F}.mp4"), None);
    }
    #[test]
    fn enclosed_alphanumerics_range_start_is_flagged() {
        assert_eq!(is_machine_dependent_filename("①.mp4"), Some('①'));
    }
    #[test]
    fn enclosed_alphanumerics_range_end_is_flagged() {
        assert_eq!(
            is_machine_dependent_filename("\u{24FF}.mp4"),
            Some('\u{24FF}')
        );
    }
    #[test]
    fn enclosed_alphanumerics_just_after_range_is_not_flagged() {
        assert_eq!(is_machine_dependent_filename("\u{2500}.mp4"), None);
    }

    // -- ローマ数字 (U+2160..=U+2188) --
    #[test]
    fn roman_numerals_just_before_range_is_not_flagged() {
        assert_eq!(is_machine_dependent_filename("\u{215F}.mp4"), None);
    }
    #[test]
    fn roman_numerals_range_start_is_flagged() {
        assert_eq!(is_machine_dependent_filename("Ⅰ.mp4"), Some('Ⅰ'));
    }
    #[test]
    fn roman_numerals_range_end_is_flagged() {
        assert_eq!(
            is_machine_dependent_filename("\u{2188}.mp4"),
            Some('\u{2188}')
        );
    }
    #[test]
    fn roman_numerals_just_after_range_is_not_flagged() {
        assert_eq!(is_machine_dependent_filename("\u{2189}.mp4"), None);
    }

    // -- 単位記号・元号合字・丸囲み漢字拡張 (U+3200..=U+33FF) --
    #[test]
    fn enclosed_cjk_just_before_range_is_not_flagged() {
        assert_eq!(is_machine_dependent_filename("\u{31FF}.mp4"), None);
    }
    #[test]
    fn enclosed_cjk_range_start_is_flagged() {
        assert_eq!(
            is_machine_dependent_filename("\u{3200}.mp4"),
            Some('\u{3200}')
        );
    }
    #[test]
    fn enclosed_cjk_range_end_is_flagged() {
        assert_eq!(
            is_machine_dependent_filename("\u{33FF}.mp4"),
            Some('\u{33FF}')
        );
    }
    #[test]
    fn enclosed_cjk_just_after_range_is_not_flagged() {
        // U+3400 is the start of CJK Unified Ideographs Extension A.
        assert_eq!(is_machine_dependent_filename("\u{3400}.mp4"), None);
    }
    #[test]
    fn circled_kanji_subset_is_flagged() {
        // U+3251..=U+32FF is a subset of the enclosed-CJK range above; spot-check it.
        assert_eq!(is_machine_dependent_filename("㉑.mp4"), Some('㉑'));
    }

    // -- トランプ柄記号 (U+2660..=U+2667) --
    #[test]
    fn card_suits_just_before_range_is_not_flagged() {
        assert_eq!(is_machine_dependent_filename("\u{265F}.mp4"), None);
    }
    #[test]
    fn card_suits_range_start_is_flagged() {
        assert_eq!(is_machine_dependent_filename("♠.mp4"), Some('♠'));
    }
    #[test]
    fn card_suits_range_end_is_flagged() {
        assert_eq!(
            is_machine_dependent_filename("\u{2667}.mp4"),
            Some('\u{2667}')
        );
    }
    #[test]
    fn card_suits_just_after_range_is_not_flagged() {
        assert_eq!(is_machine_dependent_filename("\u{2668}.mp4"), None);
    }

    // -- 外字領域 PUA (U+E000..=U+F8FF) --
    #[test]
    fn pua_just_before_range_is_not_flagged() {
        // U+D800..=U+DFFF are UTF-16 surrogates and not valid Rust `char`
        // values, so U+D7FF (the nearest representable codepoint below the
        // surrogate block) is used as the "just before PUA" boundary case.
        assert_eq!(is_machine_dependent_filename("\u{D7FF}.mp4"), None);
    }
    #[test]
    fn pua_range_start_is_flagged() {
        assert_eq!(
            is_machine_dependent_filename("\u{E000}.mp4"),
            Some('\u{E000}')
        );
    }
    #[test]
    fn pua_range_end_is_flagged() {
        assert_eq!(
            is_machine_dependent_filename("\u{F8FF}.mp4"),
            Some('\u{F8FF}')
        );
    }
    #[test]
    fn pua_just_after_range_is_not_flagged() {
        // U+F900 is the start of CJK Compatibility Ideographs.
        assert_eq!(is_machine_dependent_filename("\u{F900}.mp4"), None);
    }

    // -- 正常系 --
    #[test]
    fn ascii_filename_is_not_flagged() {
        assert_eq!(is_machine_dependent_filename("movie.mp4"), None);
    }
    #[test]
    fn japanese_filename_is_not_flagged() {
        assert_eq!(is_machine_dependent_filename("旅行の思い出.mp4"), None);
    }
    #[test]
    fn empty_string_is_not_flagged() {
        assert_eq!(is_machine_dependent_filename(""), None);
    }

    // -- 複合ケース --
    #[test]
    fn returns_the_first_offending_character_when_multiple_are_present() {
        assert_eq!(is_machine_dependent_filename("①②③.mp4"), Some('①'));
    }
    #[test]
    fn detects_offending_characters_in_the_extension() {
        assert_eq!(is_machine_dependent_filename("movie.①mp4"), Some('①'));
    }
}
