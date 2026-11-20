//! Names from a skill's author (file and folder names) as they are shown in
//! messages. JSON fields that carry a path keep it as it is; a sentence that
//! names it uses this form (owner decision 2026-11-17, sixth round).

/// `text` in the one escape form every name Kinana shows in a message shares:
/// a backslash is doubled, and every character that can make a name look
/// like another or change its line is written as `\u{..}`: control
/// characters, direction marks, line and paragraph separators, invisible and
/// format characters (soft hyphen, zero-width characters and joiners, word
/// joiner, variation selectors, tags, Hangul fillers) and every space other
/// than the plain ASCII one. Letters that merely look alike in other scripts
/// are not escaped; that is outside what a message can settle.
pub fn visible(text: &str) -> String {
    let last = text.chars().count().saturating_sub(1);
    text.chars()
        .enumerate()
        .map(|(n, character)| {
            if character == '\\' {
                "\\\\".to_owned()
            } else if character.is_control()
                || hidden_or_reordering(character)
                // A space at either end is invisible in a list.
                || (character == ' ' && (n == 0 || n == last))
            {
                character.escape_unicode().to_string()
            } else {
                character.to_string()
            }
        })
        .collect()
}

pub(crate) fn hidden_or_reordering(character: char) -> bool {
    use icu_properties::{
        CodePointMapData, CodePointSetData,
        props::{DefaultIgnorableCodePoint, GeneralCategory},
    };
    character != ' '
        && (character.is_whitespace()
            || CodePointSetData::new::<DefaultIgnorableCodePoint>().contains(character)
            || CodePointMapData::<GeneralCategory>::new().get(character) == GeneralCategory::Format
            || matches!(character, '\u{205f}'..='\u{206f}' | '\u{fff0}'..='\u{fffb}'))
}

#[cfg(test)]
mod tests {
    use super::visible;

    /// Review 2026-11-17 (sixth round), F3: these passed through unescaped and
    /// made a name display as another.
    #[test]
    fn invisible_and_format_characters_are_escaped() {
        for character in [
            '\u{00ad}',
            '\u{200c}',
            '\u{200d}',
            '\u{2060}',
            '\u{2064}',
            '\u{206a}',
            '\u{034f}',
            '\u{180e}',
            '\u{3164}',
            '\u{fff9}',
            '\u{e0041}',
            '\u{00a0}',
            '\u{2003}',
            '\u{3000}',
            '\u{fe0f}',
            '\u{202e}',
            '\u{2028}',
            '\u{1b}',
        ] {
            let shown = visible(&format!("a{character}b"));
            assert!(!shown.contains(character), "{character:?} -> {shown:?}");
            assert!(shown.starts_with("a\\u{"), "{shown:?}");
        }
        assert_eq!(visible("مهارة a b.txt"), "مهارة a b.txt");
        // Review of 4de5068: a space at either end was invisible.
        assert_eq!(visible(" lead"), "\\u{20}lead");
        assert_eq!(visible("trail "), "trail\\u{20}");
        assert_eq!(visible("a\\b"), "a\\\\b");
    }
}
