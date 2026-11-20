use crate::{Error, Result, SkillManifest};
use std::{
    fs,
    path::{Component, Path},
};
use yaml_rust2::{Yaml, YamlLoader};

/// Parse the YAML frontmatter as data. The Markdown body is never rendered or run.
pub fn parse_skill(text: &str) -> Result<SkillManifest> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Err(Error::new(
            "INVALID_FRONTMATTER",
            "SKILL.md must start with YAML frontmatter (---)",
        ));
    }
    let mut yaml = String::new();
    let mut closed = false;
    for line in lines {
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        yaml.push_str(line);
        yaml.push('\n');
    }
    if !closed {
        return Err(Error::new(
            "INVALID_FRONTMATTER",
            "Missing closing frontmatter delimiter",
        ));
    }
    let docs = YamlLoader::load_from_str(&yaml)
        .map_err(|e| Error::new("INVALID_FRONTMATTER", e.to_string()))?;
    if docs.len() != 1 || docs[0].as_hash().is_none() {
        return Err(Error::new(
            "INVALID_FRONTMATTER",
            "Frontmatter must be one YAML mapping",
        ));
    }
    let get = |key: &str| -> Result<String> {
        match &docs[0][key] {
            Yaml::String(value) if !value.trim().is_empty() => Ok(value.trim().to_string()),
            _ => Err(Error::new(
                "INVALID_FRONTMATTER",
                format!("Frontmatter requires a nonempty string '{key}'"),
            )),
        }
    };
    let name = get("name")?;
    validate_name(&name)?;
    Ok(SkillManifest {
        name,
        description: get("description")?,
    })
}

/// The shared single-component rule used by manifests and project lock names.
pub(crate) fn validate_name(name: &str) -> Result<()> {
    let mut parts = Path::new(name).components();
    if name.trim().is_empty()
        || name.trim() != name
        || !matches!(parts.next(), Some(Component::Normal(_)))
        || parts.next().is_some()
        || name.contains(['/', '\\'])
        || name.chars().any(char::is_control)
    {
        return Err(Error::new(
            "INVALID_SKILL_NAME",
            "Skill name must be a safe single directory name",
        ));
    }
    Ok(())
}

/// This function refuses a symlink in any component before opening the manifest.
pub fn read_skill(path: &Path) -> Result<SkillManifest> {
    let mut current = std::path::PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if fs::symlink_metadata(&current)?.file_type().is_symlink() {
            return Err(Error::new(
                "SYMLINK_REJECTED",
                format!("Refusing symbolic link: {}", current.display()),
            ));
        }
    }
    parse_skill(&fs::read_to_string(path)?)
}
