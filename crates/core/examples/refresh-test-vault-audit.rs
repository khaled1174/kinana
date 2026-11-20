//! Test fixture maintenance only. Recompute audit metadata with the production auditor.
use kinana_core::{audit, edit, vault::Vault};
use std::{collections::BTreeMap, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() != 2 {
        return Err("usage: refresh-test-vault-audit <vault> <isolated-index>".into());
    }
    let vault = Vault::new(PathBuf::from(&args[0]), PathBuf::from(&args[1]));
    let temporary = std::env::temp_dir().canonicalize()?;
    let root = vault.root.canonicalize()?;
    if !root.starts_with(&temporary)
        || !root.ancestors().any(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("skillsbox-phase62-"))
        })
    {
        return Err("This helper only maintains owned temporary phase-6 fixtures".into());
    }
    audit::refresh(&vault, &[], false)?;
    let mut result = BTreeMap::new();
    for record in vault.records()? {
        result.insert(
            record.id.clone(),
            edit::read_metadata(&vault, &record.id)?.audit,
        );
    }
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}
