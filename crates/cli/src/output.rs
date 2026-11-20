use kinana_core::{Error, Result};
use serde::Serialize;
use std::io::{self, Write};

pub fn emit<T: Serialize>(value: &T, json: bool, text: &str) -> Result<()> {
    let output = if json {
        serde_json::to_string(value).map_err(|e| Error::new("JSON_ERROR", e.to_string()))?
    } else {
        text.to_string()
    };
    writeln!(io::stdout().lock(), "{output}").map_err(Error::from)
}
/// Untrusted text for one column of a text line: control characters, Unicode
/// line and paragraph separators, and bidirectional embedding, override and
/// isolate controls each become one space, so the text can neither start a
/// line or column, nor drive the terminal, nor reorder its neighbours.
/// JSON output is never passed through this.
pub fn single_line(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
                )
            {
                ' '
            } else {
                character
            }
        })
        .collect()
}
pub fn warning(message: &str) -> Result<()> {
    writeln!(io::stderr().lock(), "Warning: {message}").map_err(Error::from)
}
pub fn error(error: &Error, json: bool) {
    if json {
        let value = serde_json::json!({"error": error});
        let _ = writeln!(io::stdout().lock(), "{value}");
    } else {
        let _ = writeln!(io::stderr().lock(), "{error}");
    }
}
