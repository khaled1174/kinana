//! Verification helper: isolate vault/index while retaining the provider CLI's normal authentication.
use kinana_core::{ClassifyOptions, classify, vault::Vault};
use std::path::PathBuf;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: classify-test-vault <vault> <isolated-index> <id>".into());
    }
    let vault = Vault::new(PathBuf::from(&args[0]), PathBuf::from(&args[1]));
    let report = classify::classify(
        &vault,
        &ClassifyOptions {
            ids: vec![args[2].to_str().ok_or("invalid id")?.into()],
            agent: "claude".into(),
            ..Default::default()
        },
    )?;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
