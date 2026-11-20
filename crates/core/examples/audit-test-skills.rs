//! Read-only evidence helper: print actual per-file rule strength and source positions.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut result = std::collections::BTreeMap::new();
    for path in std::env::args_os().skip(1).map(std::path::PathBuf::from) {
        result.insert(
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            kinana_core::audit::audit_skill(&path)?,
        );
    }
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
