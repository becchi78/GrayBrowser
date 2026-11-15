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

    fn flagged(c: char) -> Option<char> {
        is_machine_dependent_filename(&format!("{c}.mp4"))
    }

    #[test]
    fn each_range_flags_its_start_and_end_but_not_the_codepoints_just_outside() {
        // (range name, first flagged, last flagged, just before, just after).
        // Just-outside values are the nearest valid `char`s: for the PUA,
        // U+D800..=U+DFFF are surrogates and cannot be a `char`, so U+D7FF
        // stands in for "just before".
        let ranges = [
            (
                "Enclosed Alphanumerics",
                '\u{2460}',
                '\u{24FF}',
                '\u{245F}',
                '\u{2500}',
            ),
            (
                "Number Forms (Roman numerals)",
                '\u{2160}',
                '\u{2188}',
                '\u{215F}',
                '\u{2189}',
            ),
            // U+3400 just after is the start of CJK Unified Ideographs Extension A.
            (
                "Enclosed CJK / CJK Compatibility",
                '\u{3200}',
                '\u{33FF}',
                '\u{31FF}',
                '\u{3400}',
            ),
            ("Card suits", '\u{2660}', '\u{2667}', '\u{265F}', '\u{2668}'),
            // U+F900 just after is the start of CJK Compatibility Ideographs.
            (
                "Private Use Area",
                '\u{E000}',
                '\u{F8FF}',
                '\u{D7FF}',
                '\u{F900}',
            ),
        ];
        for (name, start, end, before, after) in ranges {
            assert_eq!(flagged(start), Some(start), "{name}: start");
            assert_eq!(flagged(end), Some(end), "{name}: end");
            assert_eq!(flagged(before), None, "{name}: just before");
            assert_eq!(flagged(after), None, "{name}: just after");
        }
    }

    #[test]
    fn well_known_machine_dependent_characters_are_flagged() {
        // Spot-checks with real characters; U+3251..=U+32FF (circled kanji)
        // is a subset of the enclosed-CJK range above.
        for c in ['①', 'Ⅰ', '㉑', '♠'] {
            assert_eq!(flagged(c), Some(c));
        }
    }

    #[test]
    fn ordinary_filenames_are_not_flagged() {
        for name in ["movie.mp4", "旅行の思い出.mp4", ""] {
            assert_eq!(is_machine_dependent_filename(name), None, "{name:?}");
        }
    }

    #[test]
    fn returns_the_first_offending_character_when_multiple_are_present() {
        assert_eq!(is_machine_dependent_filename("①②③.mp4"), Some('①'));
    }

    #[test]
    fn detects_offending_characters_in_the_extension() {
        assert_eq!(is_machine_dependent_filename("movie.①mp4"), Some('①'));
    }
}
