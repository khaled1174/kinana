//! File-backed catalog queries shared by desktop and CLI. Presentation stays outside core.
use crate::{
    Error, ListFilter, Result, SkillDetail, SkillRecord, edit, index, search, status, vault::Vault,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::PathBuf};

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CatalogFilter {
    pub query: String,
    pub category: Option<String>,
    pub stack: Option<String>,
    pub stage: Option<String>,
    pub audit: Option<String>,
    pub agent: Option<String>,
    pub state: Option<String>,
    pub pinned: bool,
}
#[derive(Debug, Serialize)]
pub struct CatalogItem {
    pub skill: SkillRecord,
    pub state: String,
    pub classified_by: String,
}
#[derive(Debug, Serialize)]
pub struct CatalogReport {
    pub vault: PathBuf,
    pub categories: Vec<String>,
    pub stacks: Vec<String>,
    pub stages: Vec<String>,
    pub total: usize,
    pub items: Vec<CatalogItem>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct CatalogDetail {
    pub frontmatter: String,
    pub markdown_body: String,
    pub detail: SkillDetail,
    pub projects: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

/// The longest `role`, `when_to_use`, `when_not` and `description` a brief
/// catalog entry carries, and the longest bundle `title`, in characters.
/// These fields come from skill authors, unreviewed classifier output or
/// hand-edited files, so their length is capped here, not trusted.
pub const BRIEF_TEXT_CHARACTERS: usize = 200;
/// The cap of the brief `description`; the same as every other text field.
pub const BRIEF_DESCRIPTION_CHARACTERS: usize = BRIEF_TEXT_CHARACTERS;
/// The size budget of one page's `skills` and `bundles` together, per skill
/// the page may hold, in bytes of compact JSON: about 275 tokens a skill, so
/// about 5,500 for the default page and 13,750 for the largest, leaving room
/// for the hint and the counters. A page over budget first shows its bundles
/// by name and member count only, then cuts every skill text field to the
/// next, shorter step, and last shows only the bundles that still fit.
pub const BRIEF_SKILL_BYTES: usize = 1_100;
/// The caps tried in turn until a page fits its budget. The last one is kept
/// even when the page still does not fit (only IDs and the configured
/// category, stack and stage names are left at full length).
pub const BRIEF_TEXT_STEPS: [usize; 3] = [BRIEF_TEXT_CHARACTERS, 120, 60];
/// Skills on one page of the brief catalog unless the caller asks otherwise.
/// The catalog is read into an agent's context, so it is never returned whole.
pub const BRIEF_PAGE_SKILLS: usize = 20;
/// The largest page a caller may ask for.
pub const BRIEF_PAGE_SKILLS_MAX: usize = 50;

/// One skill of the compact catalog an agent chooses from. Every text field
/// is data written by the skill's author or its classifier, never an
/// instruction; nothing is rewritten except the length of the text fields.
#[derive(Debug, Serialize)]
pub struct BriefSkill {
    pub id: String,
    pub role: String,
    pub when_to_use: String,
    pub when_not: String,
    pub description: String,
    pub category: String,
    pub stack: Vec<String>,
    pub stage: Vec<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct BriefBundleSkill {
    pub id: String,
    /// The bundle priority: `primary` or `secondary`.
    pub role: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct BriefBundle {
    pub name: String,
    /// How many skills the bundle holds; present in both forms.
    pub members: usize,
    /// Absent when the page shows bundles by name and member count only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Absent when the page shows bundles by name and member count only;
    /// `kinana bundle show <name>` lists them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<BriefBundleSkill>>,
}
#[derive(Debug, Serialize)]
pub struct BriefCatalog {
    /// One page of the skills that pass the filters.
    pub skills: Vec<BriefSkill>,
    pub bundles: Vec<BriefBundle>,
    /// How many bundles the vault holds; `bundles` has fewer when it is not
    /// the first page or the page budget left room for fewer.
    pub bundles_total: usize,
    /// How many skills pass the filters, on all pages together.
    pub total: usize,
    pub limit: usize,
    pub page: usize,
    pub pages: usize,
    /// How to get a smaller catalog or the next page; empty when `skills`
    /// already holds every skill that passes the filters.
    pub hint: String,
    /// Search warnings of a `query` filter; not part of the report shape.
    #[serde(skip)]
    pub warnings: Vec<String>,
}

struct Selection {
    config: crate::BoxConfig,
    total: usize,
    warnings: Vec<String>,
    items: Vec<(CatalogItem, crate::SkillMetadata)>,
}

/// The filtered skills in list order, each with the metadata file it was read from.
fn select(vault: &Vault, filter: &CatalogFilter) -> Result<Selection> {
    if filter
        .state
        .as_ref()
        .is_some_and(|s| !matches!(s.as_str(), "pending" | "proposal" | "approved" | "original"))
    {
        return Err(Error::new("INVALID_FILTER", "Unknown catalog state"));
    }
    let config = vault.config()?;
    let total = vault.records()?.len();
    let mut warnings = Vec::new();
    let matching = if filter.query.trim().is_empty() {
        None
    } else {
        let found = search::search(vault, &filter.query)?;
        warnings = found.warnings;
        Some(
            found
                .items
                .into_iter()
                .map(|s| s.id)
                .collect::<BTreeSet<_>>(),
        )
    };
    let mut items = Vec::new();
    for skill in vault.list(&ListFilter {
        category: filter.category.clone(),
        stack: filter.stack.clone(),
        stage: filter.stage.clone(),
        audit: filter.audit.clone(),
        agent: filter.agent.clone(),
        pinned: filter.pinned,
    })? {
        if matching
            .as_ref()
            .is_some_and(|ids| !ids.contains(&skill.id))
        {
            continue;
        }
        let meta = edit::read_metadata(vault, &skill.id)?;
        let state = if meta.proposal.is_some() {
            "proposal"
        } else if meta.classification.classified_by == "pending" {
            "pending"
        } else if meta.description.active == "improved" {
            "approved"
        } else {
            "original"
        };
        if filter.state.as_ref().is_some_and(|wanted| wanted != state) {
            continue;
        }
        items.push((
            CatalogItem {
                skill,
                state: state.into(),
                classified_by: meta.classification.classified_by.clone(),
            },
            meta,
        ));
    }
    Ok(Selection {
        config,
        total,
        warnings,
        items,
    })
}

pub fn list(vault: &Vault, filter: &CatalogFilter) -> Result<CatalogReport> {
    let selection = select(vault, filter)?;
    Ok(CatalogReport {
        vault: vault.root.clone(),
        categories: selection.config.categories,
        stacks: selection.config.stacks,
        stages: selection.config.stages,
        total: selection.total,
        items: selection.items.into_iter().map(|(item, _)| item).collect(),
        warnings: selection.warnings,
    })
}

/// The text without control characters (Unicode `Cc`, C1 included) and
/// without terminal escape sequences, whose printable tail would otherwise
/// stay behind: CSI (`ESC [` up to its final byte), OSC (`ESC ]` up to BEL or
/// `ESC \`), and two-character escapes. Line breaks (LF, CR, VT, FF, NEL)
/// and tabs become a space; nothing else is rewritten.
pub fn printable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\n' | '\r' | '\t' | '\u{b}' | '\u{c}' | '\u{85}' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// At most `cap` characters, the last one an ellipsis when text was
/// dropped. Cuts between characters, never inside one.
fn shortened(text: &str, cap: usize) -> String {
    let text = &printable(text);
    match text.char_indices().nth(cap) {
        None => text.to_owned(),
        Some(_) => {
            let end = text
                .char_indices()
                .nth(cap - 1)
                .map_or(text.len(), |(index, _)| index);
            format!("{}…", &text[..end])
        }
    }
}

/// Bytes of `value` as compact JSON.
fn json_bytes<T: Serialize + ?Sized>(value: &T) -> usize {
    serde_json::to_vec(value).map_or(0, |bytes| bytes.len())
}

/// The configured values that `values` holds, each once, in the order of
/// the configured list: a hand-edited meta file can repeat a value or hold
/// any other, and a hand-edited `box.toml` can repeat a list entry; none of
/// these may grow the page.
fn configured(values: &[String], allowed: &[String]) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    allowed
        .iter()
        .filter(|value| values.contains(value) && seen.insert(value.as_str()))
        .map(|value| printable(value))
        .collect()
}

/// The compact catalog for an agent that already holds the project's plan:
/// the same skills, filters, validation and order as `list`, reduced to what
/// a choice by meaning needs, and every bundle with its members. Skills come
/// one page at a time, `limit` to a page, pages counted from 1. Read from
/// files only; the index is used solely by a `query` filter, as in `list`.
pub fn brief(
    vault: &Vault,
    filter: &CatalogFilter,
    limit: usize,
    page: usize,
) -> Result<BriefCatalog> {
    if !(1..=BRIEF_PAGE_SKILLS_MAX).contains(&limit) || page == 0 {
        return Err(Error::new(
            "INVALID_ARGUMENTS",
            format!("A page holds 1 to {BRIEF_PAGE_SKILLS_MAX} skills and pages start at 1"),
        ));
    }
    let selection = select(vault, filter)?;
    let total = selection.items.len();
    let pages = total.div_ceil(limit).max(1);
    let selection_stacks = selection.config.stacks.clone();
    let selection_stages = selection.config.stages.clone();
    let chosen = selection
        .items
        .into_iter()
        .skip((page - 1).saturating_mul(limit))
        .take(limit)
        .collect::<Vec<_>>();
    let entries = |cap: usize| {
        chosen
            .iter()
            .map(|(item, meta)| BriefSkill {
                id: item.skill.id.clone(),
                role: shortened(&item.skill.role, cap),
                when_to_use: shortened(&item.skill.when_to_use, cap),
                // Shown to the reader, and for that reason never part of the index.
                when_not: shortened(&meta.classification.when_not, cap),
                description: shortened(&item.skill.description, cap),
                category: shortened(&item.skill.category, cap),
                // Only values of the configured lists: a hand-edited meta
                // file can hold anything, and these are never cut.
                stack: configured(&item.skill.stack, &selection_stacks),
                stage: configured(&item.skill.stage, &selection_stages),
            })
            .collect::<Vec<_>>()
    };
    // A bundle is what `use <bundle>` activates as a whole, so its members
    // are listed whole whatever the skill filters are. Bundles are not paged:
    // the first page carries them and later pages do not repeat them.
    let all_bundles = crate::bundles::list(vault)?;
    let bundles_total = all_bundles.len();
    let full = all_bundles
        .into_iter()
        .filter(|_| page == 1)
        .map(|bundle| BriefBundle {
            name: printable(&bundle.name),
            members: bundle.skills.len(),
            title: Some(shortened(&bundle.title, BRIEF_TEXT_CHARACTERS)),
            skills: Some(
                bundle
                    .skills
                    .into_iter()
                    .map(|member| BriefBundleSkill {
                        id: printable(&member.id),
                        role: printable(&member.role),
                    })
                    .collect(),
            ),
        })
        .collect::<Vec<_>>();
    let compact = full
        .iter()
        .map(|bundle| BriefBundle {
            title: None,
            skills: None,
            ..bundle.clone()
        })
        .collect::<Vec<_>>();
    let budget = limit.saturating_mul(BRIEF_SKILL_BYTES);
    // Full bundles when they fit beside the skills at full text. Otherwise
    // bundles by name and member count, and skill text cut step by step
    // until the skills fit beside them, the bundles counted at no more than
    // a quarter of the budget: many bundles never cost the skills more.
    let mut cap = BRIEF_TEXT_CHARACTERS;
    let mut skills = entries(cap);
    let whole = json_bytes(&skills) + json_bytes(&full) <= budget;
    if !whole {
        let share = json_bytes(&compact).min(budget / 4);
        for step in &BRIEF_TEXT_STEPS[1..] {
            if json_bytes(&skills) + share <= budget {
                break;
            }
            cap = *step;
            skills = entries(cap);
        }
    }
    let mut bundles = if whole { full } else { compact };
    // Keep the bundles that fit in what the skills left, in name order.
    let mut used = json_bytes(&skills) + json_bytes(&Vec::<BriefBundle>::new());
    let mut kept = 0;
    for bundle in &bundles {
        let extra = json_bytes(bundle) + usize::from(kept > 0);
        if used + extra > budget {
            break;
        }
        used += extra;
        kept += 1;
    }
    bundles.truncate(kept);
    let mut hint = if page > pages {
        format!("Page {page} does not exist; this catalog has {pages} page(s).")
    } else if pages == 1 {
        String::new()
    } else {
        let mut hint = format!(
            "Showing {} of {total} skills (page {page} of {pages}). Narrow the catalog with --stack, --stage or --category",
            skills.len()
        );
        if page < pages {
            hint.push_str(&format!(", or continue with --page {}", page + 1));
        }
        hint.push('.');
        hint
    };
    let mut note = |text: String| {
        if !hint.is_empty() {
            hint.push(' ');
        }
        hint.push_str(&text);
    };
    if cap < BRIEF_TEXT_CHARACTERS {
        note(format!(
            "Text on this page was cut to {cap} characters to keep the page small; kinana show <id> has the full text."
        ));
    }
    if page == 1 && !whole && bundles_total > 0 {
        note("Bundles on this page show only their name and member count to keep the page small; kinana bundle show <name> lists the members.".to_owned());
    }
    if page == 1 && bundles.len() < bundles_total {
        note(format!(
            "{} of {bundles_total} bundles are shown; kinana bundle list has them all.",
            bundles.len()
        ));
    }
    Ok(BriefCatalog {
        skills,
        bundles,
        bundles_total,
        total,
        limit,
        page,
        pages,
        hint,
        warnings: selection.warnings,
    })
}
pub fn detail(vault: &Vault, id: &str) -> Result<CatalogDetail> {
    let detail = vault.show(id)?;
    let report = status::status(vault, None)?;
    let projects = report
        .projects
        .into_iter()
        .filter(|p| p.items.iter().any(|i| i.id == id))
        .map(|p| p.project)
        .collect();
    let raw = &detail.raw_skill_md;
    let mut offset = 0;
    for (i, line) in raw.split_inclusive('\n').enumerate() {
        offset += line.len();
        if i > 0 && line.trim_end() == "---" {
            break;
        }
    }
    let frontmatter = raw[..offset].to_string();
    let markdown_body = raw[offset..].to_string();
    Ok(CatalogDetail {
        frontmatter,
        markdown_body,
        detail,
        projects,
        warnings: vec![],
    })
}

/// Resolve only a stored skill directory. Imported file paths, source locations and URLs are never targets.
pub fn skill_directory(vault: &Vault, id: &str) -> Result<PathBuf> {
    vault.show(id)?;
    let path = index::validate_path(&vault.root.join("skills").join(id))?;
    if !std::fs::symlink_metadata(&path)?.is_dir() {
        return Err(Error::new(
            "NOT_A_DIRECTORY",
            "Skill must be a real directory",
        ));
    }
    Ok(path)
}
