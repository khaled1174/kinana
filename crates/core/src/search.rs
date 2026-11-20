//! Literal, vault-scoped FTS5 search. The cache is checked against file metadata.
use crate::{Error, Result, SearchReport, vault::Vault};

pub fn search(vault: &Vault, query: &str) -> Result<SearchReport> {
    let query = query.trim();
    if query.len() > 4096 || query.contains('\0') {
        return Err(invalid());
    }
    let terms = query
        .split_whitespace()
        .filter(|term| term.chars().any(char::is_alphanumeric))
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    if terms.is_empty() || terms.len() > 32 {
        return Err(invalid());
    }
    let mut items = crate::index::search(vault, &terms.join(" AND "))?;
    let mut warnings = Vec::new();
    if items.len() > 100 {
        items.truncate(100);
        warnings
            .push("Showing the first 100 matches; refine the search to see other skills.".into());
    }
    Ok(SearchReport {
        query: query.into(),
        items,
        warnings,
    })
}

fn invalid() -> Error {
    Error::new(
        "INVALID_SEARCH_QUERY",
        "Search requires 1–32 literal terms containing letters or numbers, at most 4096 UTF-8 bytes, without NUL",
    )
}
