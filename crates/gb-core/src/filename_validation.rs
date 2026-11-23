//! Machine-dependent character detection for filenames. Pure logic: no
//! filesystem access, no `#[cfg(windows)]`.

/// Unicode ranges (inclusive) treated as machine-dependent characters.
///
/// Only the Private Use Area is listed. Windows "external characters" (gaiji)
/// registered in the character editor live there, and the glyph for a given
/// code point differs from PC to PC: on another PC or on a Mac the same name
/// shows a different character, or nothing at all. A name that cannot be read
/// back reliably elsewhere is refused.
///
/// Circled numbers, Roman numerals, unit/era ligatures, circled kanji and card
/// suits used to be listed here too. They are standard, named characters with
/// the same meaning everywhere, and the media tools open files named with them
/// fine, so refusing them only left a large part of real libraries
/// unregistered.
const MACHINE_DEPENDENT_RANGES: &[(char, char)] = &[
    ('\u{E000}', '\u{F8FF}'), // 外字領域 (Private Use Area)
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
    fn private_use_area_flags_its_start_and_end_but_not_the_codepoints_just_outside() {
        // U+D800..=U+DFFF are surrogates and cannot be a `char`, so U+D7FF
        // stands in for "just before". U+F900 is the start of CJK
        // Compatibility Ideographs.
        assert_eq!(flagged('\u{E000}'), Some('\u{E000}'), "start");
        assert_eq!(flagged('\u{F8FF}'), Some('\u{F8FF}'), "end");
        assert_eq!(flagged('\u{D7FF}'), None, "just before");
        assert_eq!(flagged('\u{F900}'), None, "just after");
    }

    #[test]
    fn standard_symbols_that_used_to_be_refused_are_not_flagged() {
        // Circled numbers, Roman numerals, circled kanji, unit and era
        // ligatures and card suits, including both ends of the old ranges.
        for c in [
            '①', '⑳', '㉑', '⓿', 'Ⅰ', 'Ⅻ', 'ↈ', '㈱', '㌔', '㍻', '㏿', '♠', '♧',
        ] {
            assert_eq!(flagged(c), None, "{c}");
        }
        assert_eq!(is_machine_dependent_filename("①②_Ⅲ㈱♠.mp4"), None);
    }

    #[test]
    fn ordinary_filenames_are_not_flagged() {
        for name in ["movie.mp4", "旅行の思い出.mp4", ""] {
            assert_eq!(is_machine_dependent_filename(name), None, "{name:?}");
        }
    }

    #[test]
    fn returns_the_first_offending_character_when_multiple_are_present() {
        assert_eq!(
            is_machine_dependent_filename("a\u{E001}b\u{E000}.mp4"),
            Some('\u{E001}')
        );
    }

    #[test]
    fn detects_offending_characters_in_the_extension() {
        assert_eq!(
            is_machine_dependent_filename("movie.\u{E000}mp4"),
            Some('\u{E000}')
        );
    }

    #[test]
    fn an_offending_character_next_to_allowed_symbols_is_still_found() {
        assert_eq!(
            is_machine_dependent_filename("①\u{E000}②.mp4"),
            Some('\u{E000}')
        );
    }
}
