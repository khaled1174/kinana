//! The latest skipped classification, stored locally and outside the vault.
use crate::activation_transaction::{ActivationTransaction, Data, PlannedWrite, Snapshot};
use crate::vault::Vault;
use crate::{ClassifyReport, Error, Result, index};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Skipped {
    pub agent: String,
    pub reason_code: String,
    pub skipped_at: String,
}

fn location(vault: &Vault) -> Result<PathBuf> {
    let root = index::validate_path(&vault.root)?;
    let scope = blake3::hash(root.as_os_str().as_encoded_bytes());
    let file = format!("classification-skipped-{scope}.json");
    index::validate_path(&vault.index_path.with_file_name(file))
}

fn decode(bytes: &[u8]) -> Result<BTreeMap<String, Skipped>> {
    serde_json::from_slice(bytes).map_err(|_| {
        Error::new(
            "CLASSIFICATION_HISTORY_INVALID",
            "Local classification history is invalid",
        )
    })
}

fn bytes(snapshot: &Snapshot) -> Result<&[u8]> {
    snapshot.bytes().ok_or_else(|| {
        Error::new(
            "CLASSIFICATION_HISTORY_INVALID",
            "Local classification history is not a file",
        )
    })
}

pub fn skipped(vault: &Vault, id: &str) -> Result<Option<Skipped>> {
    crate::vault::validate_id(id)?;
    Snapshot::read(&location(vault)?)?
        .map(|snapshot| decode(bytes(&snapshot)?).map(|mut records| records.remove(id)))
        .transpose()
        .map(Option::flatten)
}

pub(crate) fn save(vault: &Vault, report: &ClassifyReport) -> Result<()> {
    if report.skipped.is_empty() && report.items.is_empty() {
        return Ok(());
    }
    let path = location(vault)?;
    let prior = Snapshot::read(&path)?;
    let mut records = match &prior {
        Some(snapshot) => decode(bytes(snapshot)?)?,
        None => BTreeMap::new(),
    };
    let mut changed = false;
    for item in &report.items {
        changed |= records.remove(&item.id).is_some();
    }
    for item in &report.skipped {
        records.insert(
            item.id.clone(),
            Skipped {
                agent: item.agent.clone(),
                reason_code: item.reason_code.clone(),
                skipped_at: item.skipped_at.clone(),
            },
        );
        changed = true;
    }
    if !changed {
        return Ok(());
    }
    let data = serde_json::to_vec(&records).map_err(|_| {
        Error::new(
            "CLASSIFICATION_HISTORY_INVALID",
            "Local classification history could not be encoded",
        )
    })?;
    let stage_parent = path
        .parent()
        .ok_or_else(|| Error::new("INVALID_PATH", "Local state has no parent"))?
        .to_path_buf();
    let write = PlannedWrite {
        target: path,
        stage_parent,
        before: prior,
        data: Data::FileWithMode(data, 0o600),
    };
    let mut transaction = ActivationTransaction::prepare(vec![write], vec![], vec![])?;
    if let Err(error) = transaction.apply().and_then(|()| transaction.verify()) {
        return Err(transaction.abort(error));
    }
    transaction.finish();
    Ok(())
}
