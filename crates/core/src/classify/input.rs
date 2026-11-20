pub(super) struct Input {
    pub text: String,
    pub classified_from: &'static str,
}

pub(super) fn select(text: &str, full: bool) -> Input {
    let lines = text.split_inclusive('\n').collect::<Vec<_>>();
    if full || lines.len() <= 300 {
        return Input {
            text: text.into(),
            classified_from: "full",
        };
    }
    // The caller has already validated frontmatter. Preserve its original bytes.
    let frontmatter_end = lines
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, line)| line.trim_end() == "---")
        .map_or(0, |(index, _)| index + 1);
    let body = &lines[frontmatter_end..];
    let mut keep = vec![false; body.len()];
    keep.iter_mut().take(300).for_each(|keep| *keep = true);
    let mut fence: Option<(char, usize)> = None;
    let mut previous_text = false;
    for (index, line) in body.iter().enumerate() {
        let line = line.trim_end_matches(['\r', '\n']);
        let indentation = line
            .chars()
            .take_while(|character| *character == ' ')
            .count();
        let trimmed = line.trim_start_matches(' ');
        let marker = trimmed.chars().next();
        let width = marker.map_or(0, |marker| {
            trimmed
                .chars()
                .take_while(|character| *character == marker)
                .count()
        });
        if let Some((character, minimum)) = fence {
            if indentation <= 3
                && marker == Some(character)
                && width >= minimum
                && trimmed[width..].trim().is_empty()
            {
                fence = None;
            }
            previous_text = false;
            continue;
        }
        if indentation <= 3
            && matches!(marker, Some('`' | '~'))
            && width >= 3
            && (marker != Some('`') || !trimmed[width..].contains('`'))
        {
            fence = marker.map(|marker| (marker, width));
            previous_text = false;
            continue;
        }
        let atx = indentation <= 3
            && marker == Some('#')
            && (1..=6).contains(&width)
            && trimmed[width..]
                .chars()
                .next()
                .is_none_or(char::is_whitespace);
        let setext = indentation <= 3
            && previous_text
            && matches!(marker, Some('=' | '-'))
            && width > 0
            && trimmed[width..].trim().is_empty();
        if index >= 300 {
            if atx {
                keep[index] = true;
            }
            if setext {
                keep[index - 1] = true;
                keep[index] = true;
            }
        }
        previous_text = !line.trim().is_empty()
            && indentation <= 3
            && !line.starts_with('\t')
            && !atx
            && !setext;
    }
    let omitted = keep.iter().any(|keep| !keep);
    let selected = lines[..frontmatter_end]
        .iter()
        .copied()
        .chain(
            body.iter()
                .zip(keep)
                .filter_map(|(line, keep)| keep.then_some(*line)),
        )
        .collect();
    Input {
        text: selected,
        classified_from: if omitted { "excerpt" } else { "full" },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const FRONT: &str = "---\nname: sample\ndescription: Data\n---\n";

    #[test]
    fn short_files_and_exactly_300_lines_are_transmitted_byte_for_byte() {
        for count in [0, 1, 295, 296] {
            let text = format!("{FRONT}{}", "body\r\n".repeat(count));
            let input = select(&text, false);
            assert_eq!(input.text, text);
            assert_eq!(input.classified_from, "full");
        }
    }

    #[test]
    fn excerpt_keeps_frontmatter_300_body_lines_and_all_later_headings() {
        let initial = "first\n".repeat(300);
        let text = format!(
            "{FRONT}{initial}omit-one\n# Far title\nomit-two\n{}## Last title\nSetext title\n===\nomit-final\n",
            "omit\n".repeat(900)
        );
        let input = select(&text, false);
        assert_eq!(
            input.text,
            format!("{FRONT}{initial}# Far title\n## Last title\nSetext title\n===\n")
        );
        assert_eq!(input.classified_from, "excerpt");
        assert_eq!(select(&text, true).text, text);
        assert_eq!(select(&text, true).classified_from, "full");
    }

    #[test]
    fn headings_inside_fences_are_excluded_even_when_fence_begins_before_boundary() {
        let text = format!(
            "{FRONT}{}```md\n# fake\n{}## also fake\n```\n# Real\n~~~\nFake setext\n---\n~~~\nReal setext\n---\n",
            "body\n".repeat(299),
            "code\n".repeat(500)
        );
        let input = select(&text, false);
        assert!(!input.text.contains("# fake"));
        assert!(!input.text.contains("also fake"));
        assert!(!input.text.contains("Fake setext"));
        assert!(input.text.ends_with("# Real\nReal setext\n---\n"));
    }

    #[test]
    fn setext_crossing_the_boundary_does_not_duplicate_its_title() {
        let text = format!(
            "{FRONT}{}Boundary title\n===\nomitted\n",
            "body\n".repeat(299)
        );
        let input = select(&text, false);
        assert_eq!(input.text.matches("Boundary title").count(), 1);
        assert!(input.text.ends_with("Boundary title\n===\n"));
    }
}
