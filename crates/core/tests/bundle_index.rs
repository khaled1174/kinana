use kinana_core::{
    Bundle, BundleSkill, ImportOptions, Result, bundles, import, index, vault::Vault,
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().canonicalize()?.join(format!(
            "kinana-bundle-cache-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
    fn vault(&self, name: &str) -> Result<Vault> {
        let vault = Vault::new(self.0.join(name), self.0.join("cache/index.db"));
        vault.init(false)?;
        let source = self.0.join(format!("source-{name}"));
        for id in ["api-primary", "api-secondary"] {
            let directory = source.join(id);
            fs::create_dir_all(&directory)?;
            fs::write(
                directory.join("SKILL.md"),
                format!(
                    "---\nname: {id}\ndescription: Review API contracts\n---\nInert source data.\n"
                ),
            )?;
        }
        import::add(
            &vault,
            &source.to_string_lossy(),
            &self.0.join("scratch"),
            &ImportOptions {
                all: true,
                ..Default::default()
            },
        )?;
        Ok(vault)
    }
    fn bundle(&self, vault: &Vault, name: &str) -> Result<()> {
        let bundle = Bundle {
            name: name.into(),
            title: "API review".into(),
            agents: vec!["claude".into(), "codex".into()],
            skills: vec![
                BundleSkill {
                    id: "api-primary".into(),
                    role: "primary".into(),
                    group: Some("review".into()),
                },
                BundleSkill {
                    id: "api-secondary".into(),
                    role: "secondary".into(),
                    group: Some("review".into()),
                },
            ],
        };
        let file = self.0.join(format!("{}.toml", name));
        fs::write(&file, toml::to_string_pretty(&bundle)?)?;
        assert!(bundles::create(vault, &file, false)?.committed);
        Ok(())
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn rows(path: &Path) -> Result<BTreeSet<(String, String, String)>> {
    let db =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut query = db.prepare("SELECT vault,group_name,skill_id FROM groups")?;
    Ok(query
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()?)
}
#[test]
fn bundle_groups_rebuild_from_files_and_remain_scoped_to_bundle_and_vault() -> Result<()> {
    let temp = Temp::new()?;
    let first = temp.vault("first")?;
    let second = temp.vault("second")?;
    temp.bundle(&first, "web-app")?;
    temp.bundle(&first, "api-app")?;
    temp.bundle(&second, "web-app")?;
    let expected = rows(&first.index_path)?;
    assert_eq!(expected.len(), 6);
    assert_eq!(
        expected
            .iter()
            .filter(|(vault, _, _)| vault == &first.root.to_string_lossy())
            .count(),
        4
    );
    assert!(
        expected
            .iter()
            .any(|(_, group, _)| group == "web-app/review")
    );
    assert!(
        expected
            .iter()
            .any(|(_, group, _)| group == "api-app/review")
    );
    index::rebuild_for_vault(&first.index_path, &first.root, &first.records()?)?;
    assert_eq!(rows(&first.index_path)?, expected);
    fs::remove_file(&first.index_path)?;
    index::rebuild_for_vault(&first.index_path, &second.root, &second.records()?)?;
    index::rebuild_for_vault(&first.index_path, &first.root, &first.records()?)?;
    assert_eq!(rows(&first.index_path)?, expected);
    Ok(())
}
#[test]
fn invalid_file_backed_bundle_cannot_partially_replace_cached_memberships() -> Result<()> {
    let temp = Temp::new()?;
    let vault = temp.vault("vault")?;
    temp.bundle(&vault, "web-app")?;
    let before = fs::read(&vault.index_path)?;
    fs::write(
        vault.root.join("bundles/web-app.toml"),
        "name = 'web-app'\ntitle = 'Invalid'\nagents = ['claude']\nskills = [{id='missing',role='primary'}]\n",
    )?;
    assert!(index::rebuild_for_vault(&vault.index_path, &vault.root, &vault.records()?).is_err());
    assert_eq!(fs::read(&vault.index_path)?, before);
    Ok(())
}
