use kinana_core::frontmatter::parse_skill;

#[test]
fn folded_and_literal_yaml_strings_are_supported() {
    let folded = parse_skill(
        "---\nname: review\ndescription: >-\n  Review API\n  contracts.\n---\n# ignored body\n",
    )
    .unwrap();
    assert_eq!(folded.description, "Review API contracts.");
    let literal =
        parse_skill("---\nname: 'review'\ndescription: |\n  Line one\n  Line two\n---\n").unwrap();
    assert_eq!(literal.description, "Line one\nLine two");
}
#[test]
fn quoted_colons_and_unicode_are_data_not_commands() {
    let manifest = parse_skill("---\nname: مراجعة\ndescription: \"Review: API, then docs\"\n---\n<script>alert(1)</script>\n").unwrap();
    assert_eq!(manifest.name, "مراجعة");
    assert_eq!(manifest.description, "Review: API, then docs");
}
#[test]
fn incomplete_or_nonstring_frontmatter_is_rejected() {
    for text in [
        "name: x\ndescription: y\n",
        "---\nname: x\n---\n",
        "---\nname: x\ndescription: [bad\n---\n",
        "---\nname: x\ndescription: 42\n---\n",
        "---\nname: x\ndescription: ''\n---\n",
        "---\nname: x\ndescription: y\n",
        "---\n- name\n- description\n---\n",
    ] {
        assert!(parse_skill(text).is_err(), "Must reject {text:?}");
    }
}
#[test]
fn names_cannot_be_used_for_path_traversal() {
    for name in ["../outside", "..", ".", "/absolute", "a/b", "a\\b"] {
        let text = format!("---\nname: '{name}'\ndescription: Test\n---\n");
        assert!(parse_skill(&text).is_err(), "Must reject {name}");
    }
}
#[test]
fn bom_and_windows_line_endings_are_supported() {
    assert!(parse_skill("\u{feff}---\r\nname: test\r\ndescription: Test\r\n---\r\n").is_ok());
}

#[test]
fn duplicate_yaml_keys_are_rejected_instead_of_silently_overwritten() {
    assert!(parse_skill("---\nname: safe\nname: unsafe\ndescription: Test\n---\n").is_err());
}

#[test]
fn an_indented_yaml_delimiter_is_literal_description_content() {
    let manifest =
        parse_skill("---\nname: review\ndescription: |\n  First\n  ---\n  Last\n---\n").unwrap();
    assert_eq!(manifest.description, "First\n---\nLast");
}
