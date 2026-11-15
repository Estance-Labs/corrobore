// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

//! Declarative deployment of a Corrobore server (issue #388).
//!
//! A bundle is a directory with `cluster.yaml`, the policy file it names and
//! the stored queries it declares. [`Bundle::load`] reads and validates it
//! from files; [`Bundle::from_files`] does the same from an in-memory map,
//! which is how a live deployment arrives over HTTP. A bundle's
//! [`Revision`] is what gets applied: the policy text, the parsed queries
//! and a digest over all of it.
//!
//! Applied state lives under `<storage>/cluster/`: `applied.json` holds the
//! current revision, written by temporary file and rename, and
//! `deployments.jsonl` appends one line per deployment. A deployment id is
//! applied at most once: the same id with the same digest returns its
//! recorded outcome, the same id with another digest is refused.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::policy::PolicyEngine;

pub const CLUSTER_FORMAT_VERSION: u32 = 1;
pub const BUNDLE_FILE: &str = "cluster.yaml";
/// Largest bundle file accepted (each file).
pub const MAX_BUNDLE_FILE_BYTES: usize = 1024 * 1024;
/// Most stored queries one bundle declares.
pub const MAX_STORED_QUERIES: usize = 512;

#[derive(Debug, Error)]
pub enum ClusterError {
    #[error("cluster bundle is invalid: {0}")]
    Invalid(String),
    #[error("cluster state at {path} is unusable: {reason}")]
    State { path: String, reason: String },
    #[error("deployment {0} was already applied with different input")]
    DeploymentConflict(String),
}

fn invalid<T>(message: impl Into<String>) -> Result<T, ClusterError> {
    Err(ClusterError::Invalid(message.into()))
}

/// `cluster.yaml`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClusterConfig {
    pub version: u32,
    #[serde(default)]
    pub metadata: ClusterMetadata,
    /// Optional assertion: the storage root the bundle is meant for.
    #[serde(default)]
    pub storage: Option<String>,
    /// Policy file, relative to the bundle.
    #[serde(default)]
    pub policy: Option<String>,
    /// Stored queries: a directory, a list of files or directories, or
    /// explicit `name: file` entries.
    #[serde(default)]
    pub queries: Option<QueryDeclaration>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClusterMetadata {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum QueryDeclaration {
    Path(String),
    Paths(Vec<String>),
    Named(BTreeMap<String, NamedQueryFile>),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NamedQueryFile {
    pub file: String,
}

/// A validated stored query.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct StoredQuery {
    pub name: String,
    pub text: String,
    /// Whether the statement writes (judged as the engine judges `auto`).
    pub writes: bool,
    /// Parameter names the statement uses, without `$`.
    pub parameters: Vec<String>,
    /// First comment lines of the file, as a description.
    #[serde(default)]
    pub description: Option<String>,
}

/// What an apply installs.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Revision {
    pub name: Option<String>,
    pub policy: Option<String>,
    pub queries: Vec<StoredQuery>,
    /// SHA-256 over the canonical JSON of the fields above.
    pub digest: String,
}

impl Revision {
    fn seal(name: Option<String>, policy: Option<String>, mut queries: Vec<StoredQuery>) -> Self {
        queries.sort_by(|a, b| a.name.cmp(&b.name));
        let canonical = serde_json::json!({"name": name, "policy": policy, "queries": queries});
        let digest = format!(
            "sha256:{}",
            Sha256::digest(canonical.to_string().as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        Self {
            name,
            policy,
            queries,
            digest,
        }
    }

    /// The empty revision of a root that never had a deployment.
    pub fn empty() -> Self {
        Self::seal(None, None, Vec::new())
    }

    pub fn query(&self, name: &str) -> Option<&StoredQuery> {
        self.queries.iter().find(|query| query.name == name)
    }
}

/// A loaded, validated bundle.
#[derive(Clone, Debug)]
pub struct Bundle {
    pub config: ClusterConfig,
    pub revision: Revision,
}

/// Bundle files by path relative to the bundle root, `/`-separated.
pub type BundleFiles = BTreeMap<String, String>;

impl Bundle {
    /// Read `dir/cluster.yaml` and every file it names.
    pub fn load(dir: &Path) -> Result<Self, ClusterError> {
        let mut files = BundleFiles::new();
        collect_files(dir, dir, &mut files)?;
        Self::from_files(&files)
    }

    /// Validate a bundle given as files (a live deployment's body).
    pub fn from_files(files: &BundleFiles) -> Result<Self, ClusterError> {
        for (path, text) in files {
            if !safe_relative_path(path) {
                return invalid(format!("{path:?} is not a relative path inside the bundle"));
            }
            if text.len() > MAX_BUNDLE_FILE_BYTES {
                return invalid(format!(
                    "{path} is larger than {MAX_BUNDLE_FILE_BYTES} bytes"
                ));
            }
        }
        let Some(text) = files.get(BUNDLE_FILE) else {
            return invalid(format!("the bundle has no {BUNDLE_FILE}"));
        };
        let config: ClusterConfig = yaml_serde::from_str(text)
            .map_err(|error| ClusterError::Invalid(format!("{BUNDLE_FILE}: {error}")))?;
        if config.version != CLUSTER_FORMAT_VERSION {
            return invalid(format!(
                "{BUNDLE_FILE} version {} is not supported; use version {CLUSTER_FORMAT_VERSION}",
                config.version
            ));
        }
        let read = |path: &str| -> Result<&String, ClusterError> {
            let normalized = normalize(path);
            files
                .get(&normalized)
                .ok_or_else(|| ClusterError::Invalid(format!("{path} is named but missing")))
        };
        let policy = match config.policy.as_deref() {
            None => None,
            Some(path) => {
                let text = read(path)?;
                PolicyEngine::from_yaml(text)
                    .map_err(|error| ClusterError::Invalid(format!("{path}: {error}")))?;
                Some(text.clone())
            }
        };
        let mut queries = Vec::new();
        let mut names = BTreeSet::new();
        let mut add = |name: String, path: &str, text: &str| -> Result<(), ClusterError> {
            if !valid_query_name(&name) {
                return invalid(format!(
                    "{path}: query name {name:?} must be 1-64 letters, digits, _ or -"
                ));
            }
            if !names.insert(name.clone()) {
                return invalid(format!("query {name} is declared twice"));
            }
            queries.push(parse_query(name, path, text)?);
            Ok(())
        };
        match &config.queries {
            None => {}
            Some(QueryDeclaration::Named(map)) => {
                for (name, entry) in map {
                    add(name.clone(), &entry.file, read(&entry.file)?)?;
                }
            }
            Some(QueryDeclaration::Path(path)) => {
                for (name, file, text) in query_files(files, path)? {
                    add(name, &file, text)?;
                }
            }
            Some(QueryDeclaration::Paths(paths)) => {
                for path in paths {
                    for (name, file, text) in query_files(files, path)? {
                        add(name, &file, text)?;
                    }
                }
            }
        }
        if queries.len() > MAX_STORED_QUERIES {
            return invalid(format!(
                "a bundle declares at most {MAX_STORED_QUERIES} queries"
            ));
        }
        let revision = Revision::seal(config.metadata.name.clone(), policy, queries);
        Ok(Self { config, revision })
    }
}

fn normalize(path: &str) -> String {
    path.trim_start_matches("./")
        .trim_end_matches('/')
        .to_owned()
}

/// A `/`-separated relative path without `..`, `.` or empty segments.
fn safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn valid_query_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// `(name, file, text)` of a `.cypher` file, or of each top-level `.cypher`
/// file of a directory; the name is the file stem.
fn query_files<'a>(
    files: &'a BundleFiles,
    path: &str,
) -> Result<Vec<(String, String, &'a str)>, ClusterError> {
    let path = normalize(path);
    if let Some(text) = files.get(&path) {
        return Ok(vec![(stem(&path), path.clone(), text.as_str())]);
    }
    let prefix = format!("{path}/");
    let found: Vec<_> = files
        .iter()
        .filter(|(file, _)| {
            file.starts_with(&prefix)
                && !file[prefix.len()..].contains('/')
                && file.ends_with(".cypher")
        })
        .map(|(file, text)| (stem(file), file.clone(), text.as_str()))
        .collect();
    if found.is_empty() {
        return invalid(format!("{path} names no .cypher file"));
    }
    Ok(found)
}

fn stem(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.strip_suffix(".cypher").unwrap_or(file).to_owned()
}

/// Parse one stored query: it must be valid Cypher for Corrobore's parser.
fn parse_query(name: String, path: &str, text: &str) -> Result<StoredQuery, ClusterError> {
    let description = text
        .lines()
        .map_while(|line| line.trim().strip_prefix("//").map(str::trim))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    // Leading `//` lines are the description, not part of the statement.
    let statement = text
        .lines()
        .skip_while(|line| line.trim().is_empty() || line.trim().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let statement = statement.trim();
    if statement.is_empty() {
        return invalid(format!("{path} is empty"));
    }
    let validation = crate::handlers::cypher_validate::validate_query(statement);
    // A stored query outlives its author's session: a read whose structure
    // the parser does not recognise is refused, not just flagged.
    let unstructured = validation
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "UNSTRUCTURED_QUERY");
    if !validation.valid || unstructured {
        let reasons = validation
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect::<Vec<_>>()
            .join("; ");
        return invalid(format!("{path}: {reasons}"));
    }
    let mut parameters: Vec<String> = Vec::new();
    let bytes = statement.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'$' {
            let start = index + 1;
            let mut end = start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            if end > start {
                let parameter = statement[start..end].to_owned();
                if !parameters.contains(&parameter) {
                    parameters.push(parameter);
                }
            }
            index = end.max(start);
        } else {
            index += 1;
        }
    }
    Ok(StoredQuery {
        name,
        text: statement.to_owned(),
        writes: shared_runtime::contains_mutation_keywords(statement),
        parameters,
        description: (!description.is_empty()).then_some(description),
    })
}

fn collect_files(root: &Path, dir: &Path, files: &mut BundleFiles) -> Result<(), ClusterError> {
    let entries = fs::read_dir(dir).map_err(|error| {
        ClusterError::Invalid(format!("cannot read {}: {error}", dir.display()))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| ClusterError::Invalid(error.to_string()))?;
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|error| ClusterError::Invalid(error.to_string()))?;
        if kind.is_symlink() {
            return invalid(format!("{} is a symbolic link", path.display()));
        }
        if kind.is_dir() {
            collect_files(root, &path, files)?;
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|error| ClusterError::Invalid(error.to_string()))?
            .components()
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let keep = relative == BUNDLE_FILE
            || relative.ends_with(".yaml")
            || relative.ends_with(".yml")
            || relative.ends_with(".cypher");
        if !keep {
            continue;
        }
        let metadata = entry
            .metadata()
            .map_err(|error| ClusterError::Invalid(error.to_string()))?;
        if metadata.len() as usize > MAX_BUNDLE_FILE_BYTES {
            return invalid(format!(
                "{relative} is larger than {MAX_BUNDLE_FILE_BYTES} bytes"
            ));
        }
        let text = fs::read_to_string(&path)
            .map_err(|error| ClusterError::Invalid(format!("cannot read {relative}: {error}")))?;
        files.insert(relative, text);
    }
    Ok(())
}

/// One planned change between the applied and the desired revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlannedChange {
    PolicyInstall,
    PolicyUpdate,
    PolicyRemove,
    QueryAdd { name: String },
    QueryUpdate { name: String },
    QueryRemove { name: String },
}

/// What applying `desired` over `applied` changes.
pub fn plan(applied: &Revision, desired: &Revision) -> Vec<PlannedChange> {
    let mut changes = Vec::new();
    match (&applied.policy, &desired.policy) {
        (None, Some(_)) => changes.push(PlannedChange::PolicyInstall),
        (Some(_), None) => changes.push(PlannedChange::PolicyRemove),
        (Some(before), Some(after)) if before != after => changes.push(PlannedChange::PolicyUpdate),
        _ => {}
    }
    for query in &desired.queries {
        match applied.query(&query.name) {
            None => changes.push(PlannedChange::QueryAdd {
                name: query.name.clone(),
            }),
            Some(before) if before != query => changes.push(PlannedChange::QueryUpdate {
                name: query.name.clone(),
            }),
            Some(_) => {}
        }
    }
    for query in &applied.queries {
        if desired.query(&query.name).is_none() {
            changes.push(PlannedChange::QueryRemove {
                name: query.name.clone(),
            });
        }
    }
    changes
}

/// One line of the deployment ledger.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DeploymentRecord {
    pub deployment_id: String,
    pub sequence: u64,
    pub digest: String,
    pub previous_digest: String,
    pub actor: Option<String>,
    pub applied_at: String,
    pub changes: usize,
}

/// The applied revision and ledger of one storage root, or of the process
/// in ephemeral mode.
#[derive(Debug)]
pub struct ClusterState {
    dir: Option<PathBuf>,
    memory: std::sync::Mutex<(Revision, Vec<DeploymentRecord>)>,
}

impl ClusterState {
    /// State kept under `<storage_root>/cluster/`.
    pub fn on_disk(storage_root: &Path) -> Self {
        Self {
            dir: Some(storage_root.join("cluster")),
            memory: std::sync::Mutex::new((Revision::empty(), Vec::new())),
        }
    }

    /// State that lasts as long as the process.
    pub fn in_memory() -> Self {
        Self {
            dir: None,
            memory: std::sync::Mutex::new((Revision::empty(), Vec::new())),
        }
    }

    fn state_error(&self, reason: impl ToString) -> ClusterError {
        ClusterError::State {
            path: self
                .dir
                .as_ref()
                .map_or_else(|| "memory".to_owned(), |dir| dir.display().to_string()),
            reason: reason.to_string(),
        }
    }

    fn memory(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, (Revision, Vec<DeploymentRecord>)>, ClusterError> {
        self.memory
            .lock()
            .map_err(|_| self.state_error("lock poisoned"))
    }

    /// The applied revision, or the empty one.
    pub fn applied(&self) -> Result<Revision, ClusterError> {
        let Some(dir) = &self.dir else {
            return Ok(self.memory()?.0.clone());
        };
        match fs::read(dir.join("applied.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| self.state_error(error)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Revision::empty()),
            Err(error) => Err(self.state_error(error)),
        }
    }

    /// Every recorded deployment, oldest first.
    pub fn deployments(&self) -> Result<Vec<DeploymentRecord>, ClusterError> {
        let Some(dir) = &self.dir else {
            return Ok(self.memory()?.1.clone());
        };
        let text = match fs::read_to_string(dir.join("deployments.jsonl")) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(self.state_error(error)),
        };
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).map_err(|error| self.state_error(error)))
            .collect()
    }

    /// Apply `revision` as deployment `deployment_id`. Returns the record and
    /// whether this call applied it (false: an earlier identical apply).
    ///
    /// The revision is written whole and renamed into place, then the ledger
    /// line is appended. A crash between the two leaves the revision applied
    /// without its line; repeating the deployment records it.
    pub fn apply(
        &self,
        deployment_id: &str,
        revision: &Revision,
        actor: Option<String>,
    ) -> Result<(DeploymentRecord, bool), ClusterError> {
        if !valid_query_name(deployment_id) {
            return invalid("a deployment id is 1-64 letters, digits, _ or -");
        }
        let history = self.deployments()?;
        if let Some(existing) = history
            .iter()
            .find(|record| record.deployment_id == deployment_id)
        {
            return if existing.digest == revision.digest {
                Ok((existing.clone(), false))
            } else {
                Err(ClusterError::DeploymentConflict(deployment_id.to_owned()))
            };
        }
        let applied = self.applied()?;
        let record = DeploymentRecord {
            deployment_id: deployment_id.to_owned(),
            sequence: history.last().map_or(1, |record| record.sequence + 1),
            digest: revision.digest.clone(),
            previous_digest: applied.digest.clone(),
            actor,
            applied_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            changes: plan(&applied, revision).len(),
        };
        let Some(dir) = &self.dir else {
            let mut memory = self.memory()?;
            memory.0 = revision.clone();
            memory.1.push(record.clone());
            return Ok((record, true));
        };
        fs::create_dir_all(dir).map_err(|error| self.state_error(error))?;
        let temporary = dir.join("applied.json.tmp");
        let bytes = serde_json::to_vec_pretty(revision).map_err(|error| self.state_error(error))?;
        {
            let mut file = fs::File::create(&temporary).map_err(|error| self.state_error(error))?;
            file.write_all(&bytes)
                .map_err(|error| self.state_error(error))?;
            file.sync_all().map_err(|error| self.state_error(error))?;
        }
        fs::rename(&temporary, dir.join("applied.json"))
            .map_err(|error| self.state_error(error))?;
        let mut ledger = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("deployments.jsonl"))
            .map_err(|error| self.state_error(error))?;
        let line = serde_json::to_string(&record).map_err(|error| self.state_error(error))?;
        writeln!(ledger, "{line}").map_err(|error| self.state_error(error))?;
        ledger.sync_all().map_err(|error| self.state_error(error))?;
        Ok((record, true))
    }
}

/// Apply a bundle to a stopped server's root (`corrobore cluster apply`):
/// takes the root's ownership lock, so a running server refuses it.
pub fn apply_offline(
    storage_root: &Path,
    bundle: &Bundle,
    deployment_id: &str,
    actor: Option<String>,
) -> Result<(DeploymentRecord, bool, Vec<PlannedChange>), ClusterError> {
    check_storage_assertion(bundle, storage_root)?;
    let _ownership = crate::storage_ownership::DataDirectoryOwnership::acquire(storage_root)
        .map_err(|error| ClusterError::State {
            path: storage_root.display().to_string(),
            reason: error.to_string(),
        })?;
    let state = ClusterState::on_disk(storage_root);
    let before = state.applied()?;
    let (record, applied) = state.apply(deployment_id, &bundle.revision, actor)?;
    let changes = if applied {
        plan(&before, &bundle.revision)
    } else {
        Vec::new()
    };
    Ok((record, applied, changes))
}

/// A bundle that names its storage root must be applied to that root.
pub fn check_storage_assertion(bundle: &Bundle, storage_root: &Path) -> Result<(), ClusterError> {
    let Some(declared) = bundle.config.storage.as_deref() else {
        return Ok(());
    };
    let canonical = |path: &Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if canonical(Path::new(declared)) == canonical(storage_root) {
        Ok(())
    } else {
        invalid(format!(
            "the bundle is for storage {declared}, not {}",
            storage_root.display()
        ))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{Bundle, BundleFiles, ClusterState, PlannedChange, Revision, plan};

    fn files(entries: &[(&str, &str)]) -> BundleFiles {
        entries
            .iter()
            .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
            .collect()
    }

    #[test]
    fn named_and_listed_queries_are_both_accepted() {
        let named = Bundle::from_files(&files(&[
            (
                "cluster.yaml",
                "version: 1\nqueries:\n  everything: { file: q/all.cypher }\n",
            ),
            ("q/all.cypher", "MATCH (n) RETURN n"),
        ]))
        .unwrap();
        assert_eq!(named.revision.queries[0].name, "everything");
        assert!(!named.revision.queries[0].writes);
        let listed = Bundle::from_files(&files(&[
            ("cluster.yaml", "version: 1\nqueries: [a.cypher, more/]\n"),
            ("a.cypher", "CREATE (n:Note {text: $text})"),
            ("more/b.cypher", "MATCH (n:Note) RETURN n.text"),
            ("more/deeper/c.cypher", "MATCH (n) RETURN n"),
        ]))
        .unwrap();
        let names: Vec<_> = listed
            .revision
            .queries
            .iter()
            .map(|q| q.name.as_str())
            .collect();
        assert_eq!(names, ["a", "b"], "only top-level files of a directory");
        assert!(listed.revision.queries[0].writes);
        assert_eq!(listed.revision.queries[0].parameters, ["text"]);
    }

    #[test]
    fn the_digest_follows_content_only() {
        let a = Bundle::from_files(&files(&[
            ("cluster.yaml", "version: 1\nqueries: q/\n"),
            ("q/x.cypher", "MATCH (n) RETURN n"),
            ("q/y.cypher", "MATCH (m) RETURN m"),
        ]))
        .unwrap();
        let b = Bundle::from_files(&files(&[
            (
                "cluster.yaml",
                "version: 1\nqueries: [q/y.cypher, q/x.cypher]\n",
            ),
            ("q/x.cypher", "MATCH (n) RETURN n"),
            ("q/y.cypher", "MATCH (m) RETURN m"),
        ]))
        .unwrap();
        assert_eq!(a.revision.digest, b.revision.digest);
        assert_ne!(a.revision.digest, Revision::empty().digest);
    }

    #[test]
    fn plan_removes_what_the_bundle_no_longer_declares() {
        let state = ClusterState::in_memory();
        let first = Bundle::from_files(&files(&[
            ("cluster.yaml", "version: 1\nqueries: q/\n"),
            ("q/x.cypher", "MATCH (n) RETURN n"),
        ]))
        .unwrap();
        state.apply("one", &first.revision, None).unwrap();
        let empty = Bundle::from_files(&files(&[("cluster.yaml", "version: 1\n")])).unwrap();
        assert_eq!(
            plan(&state.applied().unwrap(), &empty.revision),
            [PlannedChange::QueryRemove {
                name: "x".to_owned()
            }]
        );
        assert!(state.apply("bad id!", &empty.revision, None).is_err());
    }
}
