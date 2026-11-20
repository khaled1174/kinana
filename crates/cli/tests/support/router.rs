//! A router install as an earlier release left it, for upgrade tests.
use std::{
    fs,
    path::{Path, PathBuf},
};

/// The router text shipped before 8.4, byte for byte.
pub const PREVIOUS_ROUTER: &str =
    include_str!("../../../../router-skill/superseded/SKILL-a1c4faa.md");

/// A router install exactly as the release before 8.4 left it on disk: that
/// text, and the ownership marker format that release wrote beside it (the
/// identities of the directory and both files, and the BLAKE3 of the text).
#[cfg(unix)]
pub fn install_previous_router(home: &Path, base: &str) -> PathBuf {
    use std::os::unix::fs::MetadataExt;
    #[derive(serde::Serialize)]
    struct Stamp {
        identity: String,
        mode: u32,
    }
    #[derive(serde::Serialize)]
    struct Marker {
        version: u32,
        directory: Stamp,
        skill: Stamp,
        marker: Stamp,
        skill_hash: String,
    }
    fn stamp(path: &Path) -> Stamp {
        let metadata = fs::symlink_metadata(path).unwrap();
        Stamp {
            identity: format!("{}:{}", metadata.dev(), metadata.ino()),
            mode: metadata.mode() & 0o7777,
        }
    }
    let target = home.join(base).join("skills/kinana");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("SKILL.md"), PREVIOUS_ROUTER).unwrap();
    // The marker records its own identity, so the file exists before its content.
    let marker_path = target.join(".skillsbox-router.toml");
    fs::write(&marker_path, b"").unwrap();
    let marker = Marker {
        version: 1,
        directory: stamp(&target),
        skill: stamp(&target.join("SKILL.md")),
        marker: stamp(&marker_path),
        skill_hash: "60a96d769428aa34e88507df829375c17d85e7760dc495b0d7b3a95fccaf3253".into(),
    };
    fs::write(&marker_path, toml::to_string(&marker).unwrap()).unwrap();
    target
}
