//! Private activation-only publication. Never remove a changed or replaced node.
use crate::{Error, Result, build::ActivationPayload, hash, index};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub(crate) fn directory_identity(path: &Path) -> Result<Option<String>> {
    index::validate_path(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() {
        return Ok(None);
    }
    Ok(anchor_identity(&metadata))
}

pub(crate) fn anchor_identity(metadata: &fs::Metadata) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(format!(
            "{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.mode()
        ))
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

#[derive(Clone)]
pub(crate) enum Data {
    Tree(ActivationPayload),
    Link(PathBuf),
    File(Vec<u8>),
    FileWithMode(Vec<u8>, u32),
    ProjectLock(crate::ProjectLock),
    Remove,
}

#[derive(Clone)]
enum Contents {
    Tree(String),
    Link(PathBuf),
    File(Vec<u8>),
    Directory,
}

#[derive(Clone)]
pub(crate) struct Snapshot {
    nodes: BTreeMap<PathBuf, fs::Metadata>,
    contents: Contents,
}

impl Snapshot {
    pub(crate) fn file_mode(&self) -> Option<u32> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            Some(self.nodes[Path::new("")].permissions().mode() & 0o7777)
        }
        #[cfg(not(unix))]
        {
            None
        }
    }
    pub(crate) fn directory(path: &Path) -> Result<Self> {
        index::validate_path(path)?;
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir() {
            return Err(changed("Owned directory is no longer a real directory"));
        }
        Ok(Self {
            nodes: BTreeMap::from([(PathBuf::new(), metadata)]),
            contents: Contents::Directory,
        })
    }

    pub(crate) fn ownership(&self) -> String {
        let mut hash = blake3::Hasher::new();
        for (path, metadata) in &self.nodes {
            hash.update(path.as_os_str().as_encoded_bytes());
            hash.update(&[0]);
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                hash.update(&metadata.dev().to_le_bytes());
                hash.update(&metadata.ino().to_le_bytes());
                hash.update(&metadata.mode().to_le_bytes());
            }
            hash.update(&metadata.len().to_le_bytes());
            if let Ok(time) = metadata.modified() {
                hash.update(format!("{time:?}").as_bytes());
            }
        }
        format!("blake3:{}", hash.finalize().to_hex())
    }
    pub(crate) fn evidence(&self) -> String {
        let mut hash = blake3::Hasher::new();
        hash.update(self.ownership().as_bytes());
        match &self.contents {
            Contents::Tree(digest) => hash.update(digest.as_bytes()),
            Contents::Link(path) => hash.update(path.as_os_str().as_encoded_bytes()),
            Contents::File(bytes) => hash.update(bytes),
            Contents::Directory => hash.update(b"directory"),
        };
        hash.finalize().to_hex().to_string()
    }

    pub(crate) fn matches_payload(&self, payload: &ActivationPayload) -> bool {
        if self.digest() != Some(payload.content_hash.as_str())
            || self.nodes.len() != payload.entries.len() + 1
        {
            return false;
        }
        payload.entries.iter().all(|entry| {
            self.nodes.get(&entry.relative).is_some_and(|metadata| {
                if metadata.is_file() != entry.bytes.is_some() {
                    return false;
                }
                let permissions = metadata.permissions();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    permissions.mode() & 0o7777 == entry.permissions.mode() & 0o7777
                }
                #[cfg(not(unix))]
                {
                    permissions.readonly() == entry.permissions.readonly()
                }
            })
        })
    }

    pub(crate) fn read(path: &Path) -> Result<Option<Self>> {
        index::validate_path(
            path.parent()
                .ok_or_else(|| changed("Target has no parent"))?,
        )?;
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let mut nodes = BTreeMap::from([(PathBuf::new(), metadata.clone())]);
        let contents = if metadata.file_type().is_symlink() {
            Contents::Link(fs::read_link(path)?)
        } else if metadata.is_file() {
            Contents::File(fs::read(path)?)
        } else if metadata.is_dir() {
            let digest = hash::hash_directory(path)?;
            fn visit(
                root: &Path,
                relative: &Path,
                nodes: &mut BTreeMap<PathBuf, fs::Metadata>,
            ) -> Result<()> {
                for child in fs::read_dir(root.join(relative))? {
                    let child = child?;
                    let relative = relative.join(child.file_name());
                    let path = index::validate_path(&root.join(&relative))?;
                    let metadata = fs::symlink_metadata(&path)?;
                    if !metadata.is_dir() && !metadata.is_file() {
                        return Err(changed("Special files are not activation payloads"));
                    }
                    let directory = metadata.is_dir();
                    nodes.insert(relative.clone(), metadata);
                    if directory {
                        visit(root, &relative, nodes)?;
                    }
                }
                Ok(())
            }
            visit(path, Path::new(""), &mut nodes)?;
            Contents::Tree(digest)
        } else {
            return Err(changed(format!(
                "Special activation target: {}",
                path.display()
            )));
        };
        if !same_stamp(&metadata, &fs::symlink_metadata(path)?) {
            return Err(changed("Target changed while being inspected"));
        }
        Ok(Some(Self { nodes, contents }))
    }

    pub(crate) fn digest(&self) -> Option<&str> {
        match &self.contents {
            Contents::Tree(hash) => Some(hash),
            _ => None,
        }
    }
    pub(crate) fn link(&self) -> Option<&Path> {
        match &self.contents {
            Contents::Link(path) => Some(path),
            _ => None,
        }
    }
    pub(crate) fn bytes(&self) -> Option<&[u8]> {
        match &self.contents {
            Contents::File(bytes) => Some(bytes),
            _ => None,
        }
    }

    fn same(&self, other: &Self) -> bool {
        let contents = match (&self.contents, &other.contents) {
            (Contents::Tree(left), Contents::Tree(right)) => left == right,
            (Contents::File(left), Contents::File(right)) => left == right,
            (Contents::Link(left), Contents::Link(right)) => left == right,
            _ => false,
        };
        contents
            && self.nodes.len() == other.nodes.len()
            && self.nodes.iter().all(|(path, metadata)| {
                other
                    .nodes
                    .get(path)
                    .is_some_and(|other| same_stamp(metadata, other))
            })
    }
}

pub(crate) fn matches(path: &Path, expected: Option<&Snapshot>) -> Result<bool> {
    if let Some(expected) = expected
        && matches!(expected.contents, Contents::Directory)
    {
        let actual = Snapshot::directory(path)?;
        return Ok(
            same_entry(&expected.nodes[Path::new("")], &actual.nodes[Path::new("")])
                && same_permissions(&expected.nodes[Path::new("")], &actual.nodes[Path::new("")]),
        );
    }
    if expected.is_none() {
        index::validate_path(
            path.parent()
                .ok_or_else(|| changed("Target has no parent"))?,
        )?;
        return match fs::symlink_metadata(path) {
            Ok(_) => Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(error) => Err(error.into()),
        };
    }
    Ok(match (Snapshot::read(path)?, expected) {
        (None, None) => true,
        (Some(actual), Some(expected)) => actual.same(expected),
        _ => false,
    })
}

pub(crate) struct PlannedWrite {
    pub target: PathBuf,
    pub stage_parent: PathBuf,
    pub before: Option<Snapshot>,
    pub data: Data,
}

pub(crate) fn preflight(
    plans: &[PlannedWrite],
    observations: &[(PathBuf, Option<Snapshot>)],
    anchors: &[(PathBuf, fs::Metadata)],
) -> Result<()> {
    for (path, expected) in anchors {
        index::validate_path(path)?;
        if !same_entry(expected, &fs::symlink_metadata(path)?) {
            return Err(changed("An activation anchor was replaced after planning"));
        }
    }
    for (path, expected) in observations {
        if !matches(path, expected.as_ref())? {
            return Err(changed(format!(
                "Preflight data changed: {}",
                path.display()
            )));
        }
    }
    for plan in plans {
        if !matches(&plan.target, plan.before.as_ref())? {
            return Err(changed("An activation target changed after planning"));
        }
    }
    Ok(())
}

struct PreparedWrite {
    plan: PlannedWrite,
    stage: PathBuf,
    identity: fs::Metadata,
    after: Option<Snapshot>,
    partial_files: Vec<(PathBuf, Snapshot)>,
    partial_directories: Vec<(PathBuf, fs::Metadata)>,
    backed_up: bool,
    published: bool,
}

pub(crate) struct ActivationTransaction {
    writes: Vec<PreparedWrite>,
    observations: Vec<(PathBuf, Option<Snapshot>)>,
    anchors: Vec<(PathBuf, fs::Metadata)>,
    created: Vec<(PathBuf, fs::Metadata)>,
    finished: bool,
}

impl ActivationTransaction {
    pub(crate) fn prepare(
        plans: Vec<PlannedWrite>,
        observations: Vec<(PathBuf, Option<Snapshot>)>,
        anchors: Vec<(PathBuf, fs::Metadata)>,
    ) -> Result<Self> {
        Self::prepare_guarded(plans, observations, anchors, &|_, _| Ok(()))
    }

    pub(crate) fn prepare_guarded(
        plans: Vec<PlannedWrite>,
        observations: Vec<(PathBuf, Option<Snapshot>)>,
        anchors: Vec<(PathBuf, fs::Metadata)>,
        check_stage: &impl Fn(&Path, &Path) -> Result<()>,
    ) -> Result<Self> {
        let mut transaction = Self {
            writes: Vec::new(),
            observations,
            anchors,
            created: Vec::new(),
            finished: false,
        };
        let result = (|| {
            preflight(&plans, &transaction.observations, &transaction.anchors)?;
            for mut plan in plans {
                if !matches(&plan.target, plan.before.as_ref())? {
                    return Err(changed("Target changed after preflight"));
                }
                transaction.ensure_directory(
                    plan.target
                        .parent()
                        .ok_or_else(|| changed("Target has no parent"))?,
                )?;
                transaction.ensure_directory(&plan.stage_parent)?;
                if let Data::ProjectLock(lock) = &mut plan.data {
                    let project = plan.target.parent().expect("project lock parent");
                    if let Some(restoration) = &mut lock.restoration {
                        for (path, _) in &transaction.created {
                            if let Ok(relative) = path.strip_prefix(project)
                                && let Some(relative) = relative.to_str()
                                && crate::lockfile::allowed_owned_directory(relative)
                                && let Some(identity) = directory_identity(path)?
                            {
                                restoration
                                    .directories
                                    .retain(|directory| directory.path != relative);
                                restoration.directories.push(crate::OwnedProjectDirectory {
                                    path: relative.into(),
                                    identity,
                                });
                            }
                        }
                        restoration.directories.sort_by(|a, b| a.path.cmp(&b.path));
                    }
                    for skill in &mut lock.active {
                        let target = crate::lockfile::target_path(project, skill)?;
                        if let Some(prepared) = transaction
                            .writes
                            .iter()
                            .find(|write| write.plan.target == target)
                            && let Some(snapshot) = &prepared.after
                        {
                            skill.ownership = Some(snapshot.ownership());
                        }
                    }
                    crate::lockfile::validate_for_vault(project, &lock.vault, lock)?;
                    plan.data = Data::File(crate::lockfile::serialize_preserving(project, lock)?);
                }
                let stage = loop {
                    let candidate = plan.stage_parent.join(format!(
                        ".skillsbox-activation-{}-{}",
                        std::process::id(),
                        NEXT.fetch_add(1, Ordering::Relaxed)
                    ));
                    match private_directory(&candidate) {
                        Ok(()) => break candidate,
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                        Err(error) => return Err(error.into()),
                    }
                };
                let identity = fs::symlink_metadata(&stage)?;
                transaction.writes.push(PreparedWrite {
                    plan,
                    stage,
                    identity,
                    after: None,
                    partial_files: Vec::new(),
                    partial_directories: Vec::new(),
                    backed_up: false,
                    published: false,
                });
                let last = transaction.writes.last_mut().expect("new stage");
                check_stage(&last.plan.target, &last.stage)?;
                let new = last.stage.join("new");
                match &last.plan.data {
                    Data::Tree(payload) => {
                        private_directory(&new)?;
                        last.partial_directories
                            .push((new.clone(), fs::symlink_metadata(&new)?));
                        // Only generated in-memory files and directories are copied.
                        for entry in &payload.entries {
                            let path = index::validate_path(&new.join(&entry.relative))?;
                            if let Some(bytes) = &entry.bytes {
                                write_tracked(
                                    &path,
                                    bytes,
                                    Some(&entry.permissions),
                                    false,
                                    &mut last.partial_files,
                                )?;
                            } else {
                                private_directory(&path)?;
                                last.partial_directories
                                    .push((path.clone(), fs::symlink_metadata(&path)?));
                            }
                        }
                        for entry in payload
                            .entries
                            .iter()
                            .rev()
                            .filter(|entry| entry.bytes.is_none())
                        {
                            fs::set_permissions(
                                new.join(&entry.relative),
                                entry.permissions.clone(),
                            )?;
                            if let Some((path, metadata)) = last
                                .partial_directories
                                .iter_mut()
                                .find(|(path, _)| *path == new.join(&entry.relative))
                            {
                                *metadata = fs::symlink_metadata(path)?;
                            }
                        }
                        if hash::hash_directory(&new)? != payload.content_hash {
                            return Err(changed("Staged activation payload changed"));
                        }
                    }
                    Data::Link(target) => create_link(target, &new)?,
                    Data::File(bytes) | Data::FileWithMode(bytes, _) => {
                        #[cfg(unix)]
                        let permissions = if let Data::FileWithMode(_, mode) = &last.plan.data {
                            use std::os::unix::fs::PermissionsExt;
                            Some(fs::Permissions::from_mode(*mode))
                        } else {
                            None
                        };
                        #[cfg(not(unix))]
                        let permissions = None;
                        write_tracked(
                            &new,
                            bytes,
                            permissions.as_ref(),
                            true,
                            &mut last.partial_files,
                        )?;
                    }
                    Data::Remove => continue,
                    Data::ProjectLock(_) => {
                        unreachable!("project lock was rendered after recording identities")
                    }
                }
                for (path, expected) in &last.partial_files {
                    if !matches(path, Some(expected))? {
                        return Err(changed("Staged file was replaced or modified"));
                    }
                }
                for (path, expected) in &last.partial_directories {
                    if !fs::symlink_metadata(path).is_ok_and(|actual| {
                        same_entry(expected, &actual) && same_permissions(expected, &actual)
                    }) {
                        return Err(changed("Staged directory was replaced or modified"));
                    }
                }
                if let Data::Link(target) = &last.plan.data
                    && fs::read_link(&new)? != *target
                {
                    return Err(changed("Staged link was replaced"));
                }
                last.after = Snapshot::read(&new)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            return Err(transaction.abort(error));
        }
        Ok(transaction)
    }

    fn ensure_directory(&mut self, path: &Path) -> Result<()> {
        let path = index::validate_path(path)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() => return Ok(()),
            Ok(_) => return Err(changed("Activation parent must be a directory")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        self.ensure_directory(
            path.parent()
                .ok_or_else(|| changed("Directory has no parent"))?,
        )?;
        match private_directory(&path) {
            Ok(()) => self
                .created
                .push((path.clone(), fs::symlink_metadata(&path)?)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                index::validate_path(&path)?;
                if !fs::symlink_metadata(&path)?.is_dir() {
                    return Err(changed("Activation parent appeared with a different type"));
                }
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    fn check_anchors(&self) -> Result<()> {
        for (path, expected) in self.anchors.iter().chain(self.created.iter()) {
            let path = index::validate_path(path)?;
            if !same_entry(expected, &fs::symlink_metadata(path)?) {
                return Err(changed("An activation parent or index was replaced"));
            }
        }
        Ok(())
    }

    fn check_observations(&self) -> Result<()> {
        self.check_anchors()?;
        for (path, expected) in &self.observations {
            if !matches(path, expected.as_ref())? {
                return Err(changed(format!(
                    "Preflight data changed: {}",
                    path.display()
                )));
            }
        }
        Ok(())
    }

    fn check_stage(&self, write: &PreparedWrite) -> Result<()> {
        self.check_anchors()?;
        index::validate_path(&write.stage)?;
        if !same_entry(&write.identity, &fs::symlink_metadata(&write.stage)?) {
            return Err(changed("Private activation staging was replaced"));
        }
        Ok(())
    }

    pub(crate) fn verify_staging(
        &self,
        check_stage: &impl Fn(&Path, &Path) -> Result<()>,
    ) -> Result<()> {
        for write in &self.writes {
            check_stage(&write.plan.target, &write.stage)?;
        }
        Ok(())
    }

    pub(crate) fn apply(&mut self) -> Result<()> {
        self.apply_with(|_| Ok(()))
    }

    pub(crate) fn staged_file(&self, target: &Path) -> Option<&[u8]> {
        self.writes.iter().find_map(|write| {
            if write.plan.target == target
                && let Data::File(bytes) = &write.plan.data
            {
                Some(bytes.as_slice())
            } else {
                None
            }
        })
    }

    fn apply_with(&mut self, mut after: impl FnMut(usize) -> Result<()>) -> Result<()> {
        self.check_observations()?;
        for write in &self.writes {
            if !matches(&write.plan.target, write.plan.before.as_ref())? {
                return Err(changed("Activation target changed before publication"));
            }
        }
        for index in 0..self.writes.len() {
            self.check_stage(&self.writes[index])?;
            let write = &mut self.writes[index];
            if !matches(&write.plan.target, write.plan.before.as_ref())? {
                return Err(changed("Activation target changed during publication"));
            }
            if write.plan.before.is_some() {
                if write
                    .plan
                    .before
                    .as_ref()
                    .is_some_and(|snapshot| matches!(snapshot.contents, Contents::Directory))
                    && matches!(write.plan.data, Data::Remove)
                    && fs::read_dir(&write.plan.target)?
                        .next()
                        .transpose()?
                        .is_some()
                {
                    return Err(changed("Owned directory gained contents; it was preserved"));
                }
                rename_exclusive(&write.plan.target, &write.stage.join("old"))?;
                write.backed_up = true;
                if !matches(&write.stage.join("old"), write.plan.before.as_ref())? {
                    return Err(changed(
                        "A concurrent replacement was preserved in activation staging",
                    ));
                }
            }
            if matches!(write.plan.data, Data::Remove) {
                write.published = true;
                after(index)?;
                continue;
            }
            if !matches(&write.stage.join("new"), write.after.as_ref())? {
                return Err(changed("Staged activation data changed"));
            }
            rename_exclusive(&write.stage.join("new"), &write.plan.target)?;
            write.published = true;
            after(index)?;
        }
        self.verify()
    }

    pub(crate) fn verify(&self) -> Result<()> {
        self.check_observations()?;
        for write in &self.writes {
            self.check_stage(write)?;
            if !write.published || !matches(&write.plan.target, write.after.as_ref())? {
                return Err(changed("Published activation data changed"));
            }
        }
        Ok(())
    }

    pub(crate) fn abort(&mut self, mut error: Error) -> Error {
        self.rollback();
        let warnings = self.cleanup();
        if !warnings.is_empty() {
            error
                .message
                .push_str(&format!(". {}", warnings.join("; ")));
        }
        error
    }

    pub(crate) fn finish(mut self) -> Vec<String> {
        self.finished = true;
        self.cleanup()
    }

    pub(crate) fn finish_removal(mut self, mut directories: Vec<(PathBuf, String)>) -> Vec<String> {
        self.finished = true;
        let mut warnings = self.cleanup();
        for (path, metadata) in &self.created {
            if let Some(identity) = anchor_identity(metadata) {
                directories.push((path.clone(), identity));
            }
        }
        directories
            .sort_by_key(|(path, _)| (std::cmp::Reverse(path.components().count()), path.clone()));
        directories.dedup_by(|(a, _), (b, _)| a == b);
        for (path, identity) in directories {
            match directory_identity(&path) {
                Ok(Some(actual)) if actual == identity => {
                    if let Err(error) = fs::remove_dir(&path) {
                        warnings.push(format!("Directory preserved because it is nonempty or could not be removed: {} ({error})",path.display()));
                    }
                }
                Err(error) if matches!(error.code.as_str(), "IO_ERROR") && !path.exists() => {}
                _ => warnings.push(format!(
                    "Replaced or modified directory preserved: {}",
                    path.display()
                )),
            }
        }
        warnings
    }

    fn rollback(&mut self) {
        for index in (0..self.writes.len()).rev() {
            if self.check_stage(&self.writes[index]).is_err() {
                continue;
            }
            let write = &mut self.writes[index];
            if write.published
                && matches(&write.plan.target, write.after.as_ref()).unwrap_or(false)
                && (write.after.is_none() || remove_node(&write.plan.target).is_ok())
            {
                write.published = false;
            }
            if write.backed_up
                && matches(&write.stage.join("old"), write.plan.before.as_ref()).unwrap_or(false)
                && rename_exclusive(&write.stage.join("old"), &write.plan.target).is_ok()
            {
                write.backed_up = false;
            }
        }
    }

    fn cleanup(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        for write in &self.writes {
            if !write.stage.exists() {
                continue;
            }
            if self.check_stage(write).is_ok() {
                if write.after.is_none() {
                    let mut directories = Vec::new();
                    for (path, expected) in &write.partial_directories {
                        if fs::symlink_metadata(path).is_ok_and(|actual| {
                            same_entry(expected, &actual) && same_permissions(expected, &actual)
                        }) {
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::PermissionsExt;
                                let _ = fs::set_permissions(
                                    path,
                                    fs::Permissions::from_mode(
                                        expected.permissions().mode() | 0o700,
                                    ),
                                );
                            }
                            if let Ok(metadata) = fs::symlink_metadata(path) {
                                directories.push((path, metadata));
                            }
                        }
                    }
                    for (path, expected) in &write.partial_files {
                        if matches(path, Some(expected)).unwrap_or(false) {
                            let _ = fs::remove_file(path);
                        }
                    }
                    for (path, expected) in directories.into_iter().rev() {
                        if fs::symlink_metadata(path).is_ok_and(|actual| {
                            same_entry(&expected, &actual) && same_permissions(&expected, &actual)
                        }) {
                            let _ = fs::remove_dir(path);
                        }
                    }
                }
                for (name, expected, removable) in [
                    ("new", write.after.as_ref(), true),
                    (
                        "old",
                        write.plan.before.as_ref(),
                        self.finished || !write.backed_up,
                    ),
                ] {
                    let path = write.stage.join(name);
                    if removable && expected.is_some() && matches(&path, expected).unwrap_or(false)
                    {
                        if expected.is_some_and(|snapshot| {
                            matches!(snapshot.contents, Contents::Directory)
                        }) {
                            let _ = fs::remove_dir(&path);
                        } else {
                            let _ = remove_node(&path);
                        }
                    }
                }
                if fs::remove_dir(&write.stage).is_ok() {
                    continue;
                }
            }
            warnings.push(format!(
                "Activation staging with modified, partial or unrecovered data was preserved at {}",
                write.stage.display()
            ));
        }
        if !self.finished {
            for (path, expected) in self.created.iter().rev() {
                if index::validate_path(path).is_ok()
                    && fs::symlink_metadata(path).is_ok_and(|actual| same_entry(expected, &actual))
                {
                    let _ = fs::remove_dir(path); // Empty directories only; never foreign contents.
                }
            }
        }
        warnings
    }
}

impl Drop for ActivationTransaction {
    fn drop(&mut self) {
        if !self.finished {
            self.rollback();
        }
        let _ = self.cleanup();
    }
}

fn private_directory(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

fn write_tracked(
    path: &Path,
    bytes: &[u8],
    permissions: Option<&fs::Permissions>,
    sync: bool,
    owned: &mut Vec<(PathBuf, Snapshot)>,
) -> Result<()> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    owned.push((
        path.into(),
        Snapshot {
            nodes: BTreeMap::from([(PathBuf::new(), file.metadata()?)]),
            contents: Contents::File(Vec::new()),
        },
    ));
    let snapshot = &mut owned.last_mut().expect("new owned file").1;
    let mut written = 0;
    while written < bytes.len() {
        let count = match file.write(&bytes[written..]) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if count == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::WriteZero).into());
        }
        written += count;
        snapshot.contents = Contents::File(bytes[..written].to_vec());
        snapshot.nodes.insert(PathBuf::new(), file.metadata()?);
    }
    if let Some(permissions) = permissions {
        file.set_permissions(permissions.clone())?;
    }
    snapshot.nodes.insert(PathBuf::new(), file.metadata()?);
    if sync {
        file.sync_all()?;
    }
    Ok(())
}

fn remove_node(path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        fn writable(path: &Path) -> std::io::Result<()> {
            let metadata = fs::symlink_metadata(path)?;
            if !metadata.is_dir() {
                return Ok(());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(
                    path,
                    fs::Permissions::from_mode(metadata.permissions().mode() | 0o700),
                )?;
            }
            #[cfg(not(unix))]
            {
                let mut permissions = metadata.permissions();
                permissions.set_readonly(false);
                fs::set_permissions(path, permissions)?;
            }
            for entry in fs::read_dir(path)? {
                writable(&entry?.path())?;
            }
            Ok(())
        }
        // The caller has verified the complete owned snapshot before cleanup.
        writable(path)?;
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(unix)]
fn create_link(target: &Path, path: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, path)
}
#[cfg(windows)]
fn create_link(target: &Path, path: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, path)
}
#[cfg(not(any(unix, windows)))]
fn create_link(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

fn rename_exclusive(source: &Path, target: &Path) -> Result<()> {
    crate::safe_rename::rename_no_replace(source, target).map_err(|error| {
        if matches!(
            error.kind(),
            std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::InvalidInput
        ) {
            changed(format!("Exclusive activation publication refused: {error}"))
        } else {
            error.into()
        }
    })
}

pub(crate) fn same_entry(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev() && left.ino() == right.ino()
    }
    #[cfg(not(unix))]
    {
        left.file_type() == right.file_type() && left.created().ok() == right.created().ok()
    }
}
fn same_stamp(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    same_entry(left, right)
        && left.len() == right.len()
        && left.modified().ok() == right.modified().ok()
        && same_permissions(left, right)
}

fn same_permissions(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    let permissions = left.permissions().readonly() == right.permissions().readonly();
    #[cfg(unix)]
    let permissions = {
        use std::os::unix::fs::PermissionsExt;
        permissions && left.permissions().mode() == right.permissions().mode()
    };
    permissions
}

pub(crate) fn changed(message: impl Into<String>) -> Error {
    Error::decision("ACTIVATION_CONFLICT", message)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            loop {
                let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
                    "sb-at-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("create test directory: {error}"),
                }
            }
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = remove_node(&self.0);
        }
    }
    fn file(target: PathBuf, stage_parent: PathBuf, text: &str) -> PlannedWrite {
        PlannedWrite {
            before: Snapshot::read(&target).unwrap(),
            target,
            stage_parent,
            data: Data::File(text.as_bytes().to_vec()),
        }
    }
    fn tree(root: &Path) -> ActivationPayload {
        let source = root.join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("asset"), "source bytes").unwrap();
        ActivationPayload {
            content_hash: hash::hash_directory(&source).unwrap(),
            entries: vec![crate::build::PayloadEntry {
                relative: "asset".into(),
                bytes: Some(b"source bytes".to_vec()),
                permissions: fs::metadata(source.join("asset")).unwrap().permissions(),
            }],
        }
    }

    #[test]
    fn failure_after_each_publication_restores_old_files_and_cleans_created_parents() {
        for fail_after in 0..3 {
            let temp = Temp::new();
            let project = temp.0.join("project");
            fs::create_dir(&project).unwrap();
            let old = project.join("old");
            fs::write(&old, "old bytes").unwrap();
            let old_identity = fs::metadata(&old).unwrap();
            let lock = project.join(".kinana.lock");
            fs::write(&lock, "old lock").unwrap();
            let parent = project.join(".claude/skills");
            let plans = vec![
                file(old.clone(), parent.clone(), "new bytes"),
                file(project.join("new"), parent.clone(), "created"),
                file(lock.clone(), parent, "new lock"),
            ];
            let mut transaction = ActivationTransaction::prepare(
                plans,
                vec![],
                vec![(project.clone(), fs::metadata(&project).unwrap())],
            )
            .unwrap();
            let error = transaction
                .apply_with(|index| {
                    if index == fail_after {
                        Err(changed("injected publication failure"))
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err();
            transaction.abort(error);
            drop(transaction);
            assert_eq!(fs::read_to_string(&old).unwrap(), "old bytes");
            assert!(same_entry(&old_identity, &fs::metadata(&old).unwrap()));
            assert_eq!(fs::read_to_string(&lock).unwrap(), "old lock");
            assert!(!project.join("new").exists());
            assert!(!project.join(".claude").exists());
        }
    }

    #[test]
    fn a_foreign_target_appearing_mid_batch_is_never_overwritten_or_removed() {
        let temp = Temp::new();
        let parent = temp.0.join(".claude/skills");
        let a = parent.join("a");
        let b = parent.join("b");
        let plans = vec![
            file(a.clone(), parent.clone(), "owned"),
            file(b.clone(), parent, "owned"),
        ];
        let mut transaction = ActivationTransaction::prepare(plans, vec![], vec![]).unwrap();
        let error = transaction
            .apply_with(|index| {
                if index == 0 {
                    fs::write(&b, "foreign").unwrap();
                }
                Ok(())
            })
            .unwrap_err();
        transaction.abort(error);
        drop(transaction);
        assert!(!a.exists());
        assert_eq!(fs::read_to_string(&b).unwrap(), "foreign");
    }

    #[test]
    fn an_identical_byte_child_replacement_survives_failed_tree_rollback() {
        let temp = Temp::new();
        let payload = tree(&temp.0);
        let target = temp.0.join("project/.claude/skills/item");
        let parent = target.parent().unwrap().to_path_buf();
        let plan = PlannedWrite {
            target: target.clone(),
            stage_parent: parent,
            before: None,
            data: Data::Tree(payload),
        };
        let mut transaction = ActivationTransaction::prepare(vec![plan], vec![], vec![]).unwrap();
        let saved = temp.0.join("saved-original");
        let error = transaction
            .apply_with(|_| {
                fs::rename(target.join("asset"), &saved).unwrap();
                fs::write(target.join("asset"), "source bytes").unwrap();
                Err(changed("injected user replacement"))
            })
            .unwrap_err();
        transaction.abort(error);
        drop(transaction);
        assert_eq!(
            fs::read_to_string(target.join("asset")).unwrap(),
            "source bytes"
        );
        assert_eq!(fs::read_to_string(saved).unwrap(), "source bytes");
    }

    #[test]
    fn exclusive_rename_does_not_replace_an_empty_foreign_directory() {
        let temp = Temp::new();
        let source = temp.0.join("new");
        let target = temp.0.join("foreign");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&target).unwrap();
        let identity = fs::metadata(&target).unwrap();
        assert!(rename_exclusive(&source, &target).is_err());
        assert!(source.is_dir());
        assert!(same_entry(&identity, &fs::metadata(&target).unwrap()));
    }

    #[test]
    fn removal_failures_restore_files_and_nested_directories_after_every_step() {
        for fail_after in 0..4 {
            let temp = Temp::new();
            let parent = temp.0.join("project/.claude/skills");
            fs::create_dir_all(&parent).unwrap();
            let target = parent.join("owned");
            fs::write(&target, "owned data").unwrap();
            let lock = temp.0.join("project/.kinana.lock");
            fs::write(&lock, "lock data").unwrap();
            let stage = temp.0.join("state");
            fs::create_dir(&stage).unwrap();
            let original = Snapshot::read(&target).unwrap().unwrap();
            let plans = vec![
                PlannedWrite {
                    target: target.clone(),
                    stage_parent: stage.clone(),
                    before: Some(original.clone()),
                    data: Data::Remove,
                },
                PlannedWrite {
                    target: parent.clone(),
                    stage_parent: stage.clone(),
                    before: Some(Snapshot::directory(&parent).unwrap()),
                    data: Data::Remove,
                },
                PlannedWrite {
                    target: parent.parent().unwrap().into(),
                    stage_parent: stage.clone(),
                    before: Some(Snapshot::directory(parent.parent().unwrap()).unwrap()),
                    data: Data::Remove,
                },
                PlannedWrite {
                    target: lock.clone(),
                    stage_parent: stage,
                    before: Snapshot::read(&lock).unwrap(),
                    data: Data::Remove,
                },
            ];
            let mut transaction = ActivationTransaction::prepare(plans, vec![], vec![]).unwrap();
            let error = transaction
                .apply_with(|index| {
                    if index == fail_after {
                        Err(changed("injected removal or commit failure"))
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err();
            transaction.abort(error);
            drop(transaction);
            assert!(matches(&target, Some(&original)).unwrap());
            assert_eq!(fs::read_to_string(&lock).unwrap(), "lock data");
            assert_eq!(fs::read_dir(temp.0.join("state")).unwrap().count(), 0);
        }
    }

    #[test]
    fn a_foreign_child_added_during_removal_is_preserved_and_owned_files_return() {
        let temp = Temp::new();
        let parent = temp.0.join("project/.claude/skills");
        fs::create_dir_all(&parent).unwrap();
        let target = parent.join("owned");
        fs::write(&target, "owned").unwrap();
        let stage = temp.0.join("state");
        fs::create_dir(&stage).unwrap();
        let plans = vec![
            PlannedWrite {
                target: target.clone(),
                stage_parent: stage.clone(),
                before: Snapshot::read(&target).unwrap(),
                data: Data::Remove,
            },
            PlannedWrite {
                target: parent.clone(),
                stage_parent: stage,
                before: Some(Snapshot::directory(&parent).unwrap()),
                data: Data::Remove,
            },
        ];
        let mut transaction = ActivationTransaction::prepare(plans, vec![], vec![]).unwrap();
        let error = transaction
            .apply_with(|index| {
                if index == 0 {
                    fs::write(parent.join("foreign"), "preserve").unwrap();
                }
                Ok(())
            })
            .unwrap_err();
        transaction.abort(error);
        drop(transaction);
        assert_eq!(fs::read_to_string(target).unwrap(), "owned");
        assert_eq!(
            fs::read_to_string(parent.join("foreign")).unwrap(),
            "preserve"
        );
    }

    #[test]
    fn foreign_contents_in_an_empty_directory_backup_are_never_recursively_deleted() {
        let temp = Temp::new();
        let directory = temp.0.join("empty");
        fs::create_dir(&directory).unwrap();
        let stage = temp.0.join("state");
        fs::create_dir(&stage).unwrap();
        let mut transaction = ActivationTransaction::prepare(
            vec![PlannedWrite {
                target: directory.clone(),
                stage_parent: stage,
                before: Some(Snapshot::directory(&directory).unwrap()),
                data: Data::Remove,
            }],
            vec![],
            vec![],
        )
        .unwrap();
        transaction.apply().unwrap();
        let backup = transaction.writes[0].stage.join("old/foreign");
        fs::write(&backup, "preserve").unwrap();
        let warnings = transaction.finish();
        assert!(!directory.exists());
        assert!(!warnings.is_empty());
        assert_eq!(fs::read_to_string(backup).unwrap(), "preserve");
    }

    #[test]
    fn final_empty_directory_cleanup_preserves_foreign_children() {
        let temp = Temp::new();
        let directory = temp.0.join("project/.claude/skills");
        fs::create_dir_all(&directory).unwrap();
        let identity = directory_identity(&directory).unwrap().unwrap();
        let file = directory.join("owned");
        fs::write(&file, "owned").unwrap();
        let mut transaction = ActivationTransaction::prepare(
            vec![PlannedWrite {
                target: file.clone(),
                stage_parent: directory.clone(),
                before: Snapshot::read(&file).unwrap(),
                data: Data::Remove,
            }],
            vec![],
            vec![],
        )
        .unwrap();
        transaction.apply().unwrap();
        fs::write(directory.join("foreign"), "keep").unwrap();
        let warnings = transaction.finish_removal(vec![(directory.clone(), identity)]);
        assert!(!file.exists());
        assert!(!warnings.is_empty());
        assert_eq!(
            fs::read_to_string(directory.join("foreign")).unwrap(),
            "keep"
        );
    }

    #[test]
    fn adoption_failure_restores_the_original_directory_and_its_identity() {
        let temp = Temp::new();
        let target = temp.0.join("project/.claude/skills/item");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("SKILL.md"), "original bytes").unwrap();
        let original = Snapshot::read(&target).unwrap().unwrap();
        let mut transaction = ActivationTransaction::prepare(
            vec![PlannedWrite {
                target: target.clone(),
                stage_parent: target.parent().unwrap().into(),
                before: Some(original.clone()),
                data: Data::Link(temp.0.join("generated")),
            }],
            vec![],
            vec![],
        )
        .unwrap();
        let error = transaction
            .apply_with(|_| Err(changed("injected failure after adoption publication")))
            .unwrap_err();
        transaction.abort(error);
        drop(transaction);
        assert!(matches(&target, Some(&original)).unwrap());
        assert_eq!(
            fs::read_to_string(target.join("SKILL.md")).unwrap(),
            "original bytes"
        );
    }

    #[test]
    fn a_replaced_parent_is_not_followed_during_rollback() {
        let temp = Temp::new();
        let parent = temp.0.join("project/.claude/skills");
        let target = parent.join("item");
        let mut transaction = ActivationTransaction::prepare(
            vec![file(target, parent.clone(), "owned")],
            vec![],
            vec![],
        )
        .unwrap();
        let moved = temp.0.join("moved");
        let error = transaction
            .apply_with(|_| {
                fs::rename(parent.parent().unwrap(), &moved).unwrap();
                fs::create_dir_all(&parent).unwrap();
                fs::write(parent.join("sentinel"), "foreign").unwrap();
                Err(changed("injected parent replacement"))
            })
            .unwrap_err();
        transaction.abort(error);
        drop(transaction);
        assert_eq!(
            fs::read_to_string(parent.join("sentinel")).unwrap(),
            "foreign"
        );
        assert_eq!(
            fs::read_to_string(moved.join("skills/item")).unwrap(),
            "owned"
        );
    }

    #[test]
    fn partial_preparation_cleans_only_files_it_created() {
        let temp = Temp::new();
        let mut payload = tree(&temp.0);
        let mut invalid = payload.entries[0].clone();
        invalid.relative = "missing/asset".into();
        payload.entries.push(invalid);
        let parent = temp.0.join("project/.claude/skills");
        let target = parent.join("item");
        assert!(
            ActivationTransaction::prepare(
                vec![PlannedWrite {
                    target,
                    stage_parent: parent,
                    before: None,
                    data: Data::Tree(payload)
                }],
                vec![],
                vec![]
            )
            .is_err()
        );
        assert!(!temp.0.join("project").exists());
    }

    #[test]
    fn partial_staging_preserves_foreign_data_and_reports_its_location() {
        let temp = Temp::new();
        let payload = tree(&temp.0);
        let parent = temp.0.join("project/.claude/skills");
        let mut transaction = ActivationTransaction::prepare(
            vec![PlannedWrite {
                target: parent.join("item"),
                stage_parent: parent,
                before: None,
                data: Data::Tree(payload),
            }],
            vec![],
            vec![],
        )
        .unwrap();
        let stage = transaction.writes[0].stage.clone();
        // Model an error after the owned files were written but before the
        // completed tree snapshot could be recorded.
        transaction.writes[0].after = None;
        fs::write(stage.join("new/foreign"), "keep foreign bytes").unwrap();
        let error = transaction.abort(changed("injected preparation failure"));
        drop(transaction);
        assert!(error.message.contains(&stage.display().to_string()));
        assert_eq!(
            fs::read_to_string(stage.join("new/foreign")).unwrap(),
            "keep foreign bytes"
        );
        assert!(!stage.join("new/asset").exists());
    }

    #[cfg(unix)]
    #[test]
    fn readonly_generated_directories_do_not_prevent_owned_tree_rollback() {
        use std::os::unix::fs::PermissionsExt;
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir_all(source.join("readonly")).unwrap();
        fs::write(source.join("readonly/asset"), "data").unwrap();
        fs::set_permissions(source.join("readonly"), fs::Permissions::from_mode(0o555)).unwrap();
        let payload = ActivationPayload {
            content_hash: hash::hash_directory(&source).unwrap(),
            entries: vec![
                crate::build::PayloadEntry {
                    relative: "readonly".into(),
                    bytes: None,
                    permissions: fs::metadata(source.join("readonly")).unwrap().permissions(),
                },
                crate::build::PayloadEntry {
                    relative: "readonly/asset".into(),
                    bytes: Some(b"data".to_vec()),
                    permissions: fs::metadata(source.join("readonly/asset"))
                        .unwrap()
                        .permissions(),
                },
            ],
        };
        let parent = temp.0.join("project/.claude/skills");
        let target = parent.join("item");
        let mut transaction = ActivationTransaction::prepare(
            vec![PlannedWrite {
                target: target.clone(),
                stage_parent: parent,
                before: None,
                data: Data::Tree(payload),
            }],
            vec![],
            vec![],
        )
        .unwrap();
        let error = transaction
            .apply_with(|_| Err(changed("injected failure")))
            .unwrap_err();
        transaction.abort(error);
        drop(transaction);
        assert!(!target.exists());
        assert!(!temp.0.join("project").exists());
        assert_eq!(
            fs::metadata(source.join("readonly"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o555
        );
    }
}
