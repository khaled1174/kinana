//! Stable presentation reasons. Legacy CLI messages stay diagnostic text only;
//! desktop callers receive codes and never infer them by parsing English strings.
use crate::{Error, Result};
use serde::Serialize;
use serde_json::Value;
fn encoded<T: Serialize>(value: &T, codes: Vec<&str>) -> Result<Value> {
    let mut value =
        serde_json::to_value(value).map_err(|e| Error::new("JSON_ERROR", e.to_string()))?;
    value
        .as_object_mut()
        .ok_or_else(|| Error::new("JSON_ERROR", "Report must be an object"))?
        .insert(
            "diagnostic_codes".into(),
            serde_json::to_value(codes).unwrap(),
        );
    Ok(value)
}
pub fn catalog(report: &crate::catalog::CatalogReport) -> Result<Value> {
    encoded(
        report,
        if report.warnings.is_empty() {
            vec![]
        } else {
            vec!["SEARCH_INDEX_FALLBACK"]
        },
    )
}
pub fn detail(report: &crate::catalog::CatalogDetail) -> Result<Value> {
    encoded(
        report,
        if report.warnings.is_empty() {
            vec![]
        } else {
            vec!["PROJECT_INSPECTION_NOTICE"]
        },
    )
}
pub fn groups(report: &crate::GroupsReport) -> Result<Value> {
    let mut value = encoded(
        report,
        if report.warnings.is_empty() {
            vec![]
        } else {
            vec!["OVERLAP_REFERENCE_SKIPPED"]
        },
    )?;
    for candidate in value["candidates"].as_array_mut().into_iter().flatten() {
        candidate.as_object_mut().unwrap().insert(
            "reason_code".into(),
            Value::String("OVERLAP_CANDIDATE".into()),
        );
    }
    Ok(value)
}
pub fn classification(report: &crate::ClassifyReport) -> Result<Value> {
    let mut codes = vec![];
    if report.dry_run {
        codes.push("CLASSIFICATION_PREVIEW_ONLY");
    }
    if report.items.iter().any(|i| i.attempts > 1) || report.skipped.iter().any(|i| i.attempts > 1)
    {
        codes.push("CLASSIFICATION_RETRIED");
    }
    if report
        .items
        .iter()
        .any(|i| i.classified_by == "keywords" && !i.warnings.is_empty())
    {
        codes.push("CLASSIFICATION_KEYWORD_FALLBACK");
    }
    encoded(report, codes)
}
pub fn import(report: &crate::ImportReport) -> Result<Value> {
    let mut codes = vec![];
    if report.requires_preparation {
        codes.push("SOURCE_PREPARATION_REQUIRED");
    }
    if !report.warnings.is_empty() {
        codes.push("SOURCE_INSPECTION_NOTICE");
    }
    let mut value = encoded(report, codes)?;
    for (item, source) in value["items"]
        .as_array_mut()
        .into_iter()
        .flatten()
        .zip(&report.items)
    {
        item["review_token"] = source.review_token.clone().into();
        item["description"] = source.description.clone().into();
        let code = if item["error"].is_null() {
            None
        } else {
            Some("IMPORT_ITEM_INVALID")
        };
        item.as_object_mut()
            .unwrap()
            .insert("reason_code".into(), code.into());
    }
    Ok(value)
}
pub fn project(report: &crate::projects::Project) -> Result<Value> {
    encoded(
        report,
        if report.warnings.is_empty() {
            vec![]
        } else {
            vec!["PROJECT_INSPECTION_NOTICE"]
        },
    )
}
pub fn project_plan(report: &crate::projects::Preview) -> Result<Value> {
    encoded(
        report,
        if report.warnings.is_empty() {
            vec![]
        } else {
            vec!["PROJECT_PUBLICATION_NOTICE"]
        },
    )
}
pub fn bundle(report: &crate::BundleSaveReport) -> Result<Value> {
    encoded(
        report,
        if report.warnings.is_empty() {
            vec![]
        } else {
            vec!["BUNDLE_PUBLICATION_NOTICE"]
        },
    )
}
