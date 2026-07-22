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
use std::fs;
use std::path::{Path, PathBuf};

use crate::{
    GraphId, GraphStorageError, GraphStorageResult, RecordFormat, StorageManifest,
    StorageTimestamp, StorageVersion,
};

const MANIFEST_FILE_NAME: &str = "manifest.json";

/// Filesystem root for one local persistent graph store.
///
///
/// - Represent the boundary between a logical graph store and its local storage
///   location.
/// - Keep path ownership inside the storage crate rather than graph-core.
/// - Provide a stable handle that later create/open/read/validate behavior can
///   build on without loading a full graph into memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageRoot {
    /// Local filesystem path that owns the manifest, catalog, logs, adjacency, and
    /// future snapshot directories for one graph store.
    pub path: PathBuf,
}

impl StorageRoot {
    /// Return the root path as a borrowed path.
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    fn manifest_path(&self) -> PathBuf {
        self.path.join(MANIFEST_FILE_NAME)
    }
}

/// Create a new storage root with an initial manifest.
///
///
/// - Initialize a persistent graph store root without coupling graph-core to the
///   filesystem layout.
/// - Keep manifest creation separate from graph construction.
/// - Avoid loading graph records while creating the storage boundary.
///
///
///   1. Reject an existing storage root with `StorageRootAlreadyExists`.
/// 2. Validate the provided manifest before writing it.
/// 3. Create the root directory.
/// 4. Write the manifest.
/// 5. Return a `StorageRoot` handle without loading graph records.
pub fn create_storage_root(
    path: impl Into<PathBuf>,
    manifest: StorageManifest,
) -> GraphStorageResult<StorageRoot> {
    let path = path.into();

    if path.exists() {
        return Err(GraphStorageError::StorageRootAlreadyExists { path });
    }

    validate_storage_manifest(&manifest)?;

    fs::create_dir_all(&path).map_err(|error| GraphStorageError::OperationFailed {
        operation: "create_storage_root",
        message: error.to_string(),
    })?;

    let root = StorageRoot { path };
    write_storage_manifest(&root, &manifest)?;

    Ok(root)
}

/// Open an existing storage root.
///
///
/// - Reopen a persisted graph store from its manifest boundary.
/// - Validate compatibility before catalog, node, relationship, or adjacency data
///   is loaded.
/// - Leave catalog loading/rebuild and persistent pager setup to later issues.
///
///
///   1. Reject a missing root with `StorageRootNotFound`.
/// 2. Read and validate the manifest.
/// 3. Return a `StorageRoot` handle that can later be used by catalog and pager
///    code.
/// 4. Avoid loading node payloads, relationship payloads, or adjacency payloads as
///    part of open.
pub fn open_storage_root(path: impl Into<PathBuf>) -> GraphStorageResult<StorageRoot> {
    let path = path.into();

    if !path.is_dir() {
        return Err(GraphStorageError::StorageRootNotFound { path });
    }

    let root = StorageRoot { path };
    let manifest = read_storage_manifest(&root)?;
    validate_storage_manifest(&manifest)?;

    Ok(root)
}

/// Read the manifest from a storage root.
///
///
/// - Keep manifest reads independent from catalog or record reads.
/// - Provide the focused unit surface for missing and corrupted manifests.
/// - Keep manifest IO outside graph-core.
///
///
///   Return `ManifestNotFound` when the manifest file is absent and
///   `ManifestCorrupted` when the manifest cannot be decoded safely.
pub fn read_storage_manifest(root: &StorageRoot) -> GraphStorageResult<StorageManifest> {
    let manifest_path = root.manifest_path();

    if !manifest_path.is_file() {
        return Err(GraphStorageError::ManifestNotFound {
            path: manifest_path,
        });
    }

    let content =
        fs::read_to_string(&manifest_path).map_err(|error| GraphStorageError::OperationFailed {
            operation: "read_storage_manifest",
            message: error.to_string(),
        })?;

    parse_storage_manifest(&content).map_err(|reason| GraphStorageError::ManifestCorrupted {
        path: manifest_path,
        reason,
    })
}

/// Validate manifest compatibility and required metadata.
///
///
/// - Validate compatibility before any graph records are loaded.
/// - Make storage version and record format checks explicit.
/// - Keep missing, corrupted, unsupported, and inconsistent manifest handling
///   deterministic for callers.
///
///
/// Return `UnsupportedStorageVersion` for incompatible storage versions,
/// `UnsupportedRecordFormat` for incompatible record formats, and
/// `InvalidManifest` for missing or inconsistent required metadata.
pub fn validate_storage_manifest(manifest: &StorageManifest) -> GraphStorageResult<()> {
    match &manifest.storage_version {
        StorageVersion::V1 => {}
        StorageVersion::Unsupported(version) => {
            return Err(GraphStorageError::UnsupportedStorageVersion {
                version: version.clone(),
            });
        }
    }

    match &manifest.record_format {
        RecordFormat::JsonLinesV1 => {}
        RecordFormat::Unsupported(format) => {
            return Err(GraphStorageError::UnsupportedRecordFormat {
                format: format.clone(),
            });
        }
    }

    if manifest.graph_id.value.trim().is_empty() {
        return Err(GraphStorageError::InvalidManifest {
            reason: "graph_id must not be empty".to_owned(),
        });
    }

    if manifest.created_at.value.trim().is_empty() {
        return Err(GraphStorageError::InvalidManifest {
            reason: "created_at must not be empty".to_owned(),
        });
    }

    if manifest.updated_at.value.trim().is_empty() {
        return Err(GraphStorageError::InvalidManifest {
            reason: "updated_at must not be empty".to_owned(),
        });
    }

    Ok(())
}

fn write_storage_manifest(
    root: &StorageRoot,
    manifest: &StorageManifest,
) -> GraphStorageResult<()> {
    let manifest_path = root.manifest_path();
    let content = format_storage_manifest(manifest);

    fs::write(&manifest_path, content).map_err(|error| GraphStorageError::OperationFailed {
        operation: "write_storage_manifest",
        message: error.to_string(),
    })
}

fn format_storage_manifest(manifest: &StorageManifest) -> String {
    format!(
        concat!(
            "{{\n",
            " \"storage_version\": \"{}\",\n",
            " \"graph_id\": {{ \"value\": \"{}\" }},\n",
            " \"created_at\": {{ \"value\": \"{}\" }},\n",
            " \"updated_at\": {{ \"value\": \"{}\" }},\n",
            " \"record_format\": \"{}\"\n",
            "}}\n"
        ),
        storage_version_as_str(&manifest.storage_version),
        escape_json_string(&manifest.graph_id.value),
        escape_json_string(&manifest.created_at.value),
        escape_json_string(&manifest.updated_at.value),
        record_format_as_str(&manifest.record_format),
    )
}

fn parse_storage_manifest(content: &str) -> Result<StorageManifest, String> {
    Ok(StorageManifest {
        storage_version: parse_storage_version(required_top_level_string(
            content,
            "storage_version",
        )?),
        graph_id: GraphId {
            value: required_nested_value(content, "graph_id")?,
        },
        created_at: StorageTimestamp {
            value: required_nested_value(content, "created_at")?,
        },
        updated_at: StorageTimestamp {
            value: required_nested_value(content, "updated_at")?,
        },
        record_format: parse_record_format(required_top_level_string(content, "record_format")?),
    })
}

fn required_top_level_string(content: &str, key: &str) -> Result<String, String> {
    let marker = format!("\"{key}\"");
    let key_index = content
        .find(&marker)
        .ok_or_else(|| format!("missing manifest field `{key}`"))?;
    let after_key = &content[key_index + marker.len()..];
    let colon_index = after_key
        .find(':')
        .ok_or_else(|| format!("missing colon after manifest field `{key}`"))?;
    let after_colon = after_key[colon_index + 1..].trim_start();

    parse_json_string(after_colon)
        .ok_or_else(|| format!("manifest field `{key}` must be encoded as a JSON string"))
}

fn required_nested_value(content: &str, key: &str) -> Result<String, String> {
    let marker = format!("\"{key}\"");
    let key_index = content
        .find(&marker)
        .ok_or_else(|| format!("missing manifest field `{key}`"))?;
    let after_key = &content[key_index + marker.len()..];
    let value_marker = "\"value\"";
    let value_index = after_key
        .find(value_marker)
        .ok_or_else(|| format!("missing `value` for manifest field `{key}`"))?;
    let after_value = &after_key[value_index + value_marker.len()..];
    let colon_index = after_value
        .find(':')
        .ok_or_else(|| format!("missing colon after `{key}.value`"))?;
    let after_colon = after_value[colon_index + 1..].trim_start();

    parse_json_string(after_colon)
        .ok_or_else(|| format!("manifest field `{key}.value` must be encoded as a JSON string"))
}

fn parse_json_string(input: &str) -> Option<String> {
    let mut chars = input.chars();

    if chars.next()? != '"' {
        return None;
    }

    let mut value = String::new();
    let mut escaped = false;

    for character in chars {
        if escaped {
            match character {
                '"' => value.push('"'),
                '\\' => value.push('\\'),
                'n' => value.push('\n'),
                'r' => value.push('\r'),
                't' => value.push('\t'),
                other => value.push(other),
            }
            escaped = false;
            continue;
        }

        match character {
            '\\' => escaped = true,
            '"' => return Some(value),
            other => value.push(other),
        }
    }

    None
}

fn parse_storage_version(value: String) -> StorageVersion {
    match value.as_str() {
        "V1" => StorageVersion::V1,
        _ => StorageVersion::Unsupported(value),
    }
}

fn parse_record_format(value: String) -> RecordFormat {
    match value.as_str() {
        "JsonLinesV1" => RecordFormat::JsonLinesV1,
        _ => RecordFormat::Unsupported(value),
    }
}

fn storage_version_as_str(version: &StorageVersion) -> &str {
    match version {
        StorageVersion::V1 => "V1",
        StorageVersion::Unsupported(value) => value.as_str(),
    }
}

fn record_format_as_str(format: &RecordFormat) -> &str {
    match format {
        RecordFormat::JsonLinesV1 => "JsonLinesV1",
        RecordFormat::Unsupported(value) => value.as_str(),
    }
}

fn escape_json_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_path(test_name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "intelligence_graph_engine_root_tests_{test_name}_{}_{}",
            std::process::id(),
            unique
        ))
    }

    fn manifest_with(graph_id: &str, created_at: &str, updated_at: &str) -> StorageManifest {
        StorageManifest {
            // Storage version.
            storage_version: StorageVersion::V1,
            // Graph id.
            graph_id: GraphId {
                // Value.
                value: graph_id.to_owned(),
            },
            // Created at.
            created_at: StorageTimestamp {
                // Value.
                value: created_at.to_owned(),
            },
            // Updated at.
            updated_at: StorageTimestamp {
                // Value.
                value: updated_at.to_owned(),
            },
            // Record format.
            record_format: RecordFormat::JsonLinesV1,
        }
    }

    #[test]
    fn create_storage_root_writes_manifest_and_open_storage_root_reuses_it() {
        let path = unique_temp_path("create_and_open_roundtrip");
        let _ = fs::remove_dir_all(&path);
        let manifest = manifest_with(
            "graph--roundtrip",
            "2026-01-01T00:00:00Z",
            "2026-01-02T00:00:00Z",
        );

        let root = create_storage_root(path.clone(), manifest.clone())
            .expect("storage root should be created for valid manifest");
        assert!(root.path().is_dir());

        let read_back =
            read_storage_manifest(&root).expect("manifest should be readable after root creation");
        assert_eq!(read_back, manifest);

        let reopened = open_storage_root(path.clone())
            .expect("open_storage_root should accept valid root and manifest");
        assert_eq!(reopened.path(), path.as_path());

        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn create_storage_root_rejects_existing_path() {
        let path = unique_temp_path("reject_existing_path");
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("fixture path should be created");

        let error = create_storage_root(
            path.clone(),
            manifest_with("graph--id", "2026-01-01T00:00:00Z", "2026-01-02T00:00:00Z"),
        )
        .expect_err("existing root path should be rejected");

        assert!(matches!(
        error,
        GraphStorageError::StorageRootAlreadyExists { path: actual } if actual == path
        ));

        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn open_storage_root_rejects_missing_root() {
        let path = unique_temp_path("missing_root");
        let _ = fs::remove_dir_all(&path);

        let error =
            open_storage_root(path.clone()).expect_err("missing root directory should fail open");

        assert!(matches!(
        error,
        GraphStorageError::StorageRootNotFound { path: actual } if actual == path
        ));
    }

    #[test]
    fn read_storage_manifest_reports_missing_manifest_file() {
        let path = unique_temp_path("missing_manifest_file");
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("fixture root should exist");
        let root = StorageRoot { path: path.clone() };

        let error =
            read_storage_manifest(&root).expect_err("missing manifest file should be reported");

        assert!(matches!(
        error,
        GraphStorageError::ManifestNotFound { path: manifest_path }
        if manifest_path == path.join("manifest.json")
        ));

        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn read_storage_manifest_reports_corrupted_manifest_content() {
        let path = unique_temp_path("corrupted_manifest");
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("fixture root should exist");
        fs::write(path.join("manifest.json"), "{not-json")
            .expect("invalid manifest fixture should be written");
        let root = StorageRoot { path: path.clone() };

        let error =
            read_storage_manifest(&root).expect_err("corrupted manifest should be rejected");

        assert!(matches!(error, GraphStorageError::ManifestCorrupted { .. }));

        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn open_storage_root_rejects_invalid_manifest_after_read() {
        let path = unique_temp_path("invalid_manifest_on_open");
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("fixture root should exist");
        fs::write(
            path.join("manifest.json"),
            r#"{
 "storage_version": "V1",
 "graph_id": { "value": "graph--invalid-open" },
 "created_at": { "value": "2026-01-01T00:00:00Z" },
 "updated_at": { "value": "2026-01-02T00:00:00Z" },
 "record_format": "BinaryV1"
}
"#,
        )
        .expect("manifest fixture should be written");

        let error = open_storage_root(path.clone())
            .expect_err("unsupported record format should fail open");

        assert!(matches!(
        error,
        GraphStorageError::UnsupportedRecordFormat { format } if format == "BinaryV1"
        ));

        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn parse_json_string_decodes_supported_escape_sequences() {
        let parsed = parse_json_string("\"line\\nquote:\\\" tab:\\t slash:\\\\\" trailing")
            .expect("escaped JSON string should parse");

        assert_eq!(parsed, "line\nquote:\" tab:\t slash:\\");
    }

    #[test]
    fn parse_json_string_rejects_non_string_and_unterminated_input() {
        assert_eq!(parse_json_string("123"), None);
        assert_eq!(parse_json_string("\"unterminated"), None);
    }

    #[test]
    fn required_top_level_string_requires_json_string_value() {
        let content = r#"{
 "storage_version": 7
}"#;

        let error = required_top_level_string(content, "storage_version")
            .expect_err("non-string values should be rejected");

        assert_eq!(
            error,
            "manifest field `storage_version` must be encoded as a JSON string"
        );
    }

    #[test]
    fn required_nested_value_reports_missing_value_field() {
        let content = r#"{
 "graph_id": {}
}"#;

        let error = required_nested_value(content, "graph_id")
            .expect_err("missing nested value field should be reported");

        assert_eq!(error, "missing `value` for manifest field `graph_id`");
    }

    #[test]
    fn parse_storage_manifest_reports_missing_fields() {
        let content = r#"{
 "storage_version": "V1",
 "graph_id": { "value": "graph--id" },
 "created_at": { "value": "2026-01-01T00:00:00Z" }
}"#;

        let error =
            parse_storage_manifest(content).expect_err("missing record_format should be surfaced");

        assert_eq!(error, "missing manifest field `updated_at`");
    }

    #[test]
    fn format_and_parse_manifest_round_trip_preserves_special_characters() {
        let original = manifest_with(
            "graph-\\\"quoted\\\"-id",
            "2026-01-01T00:00:00Z",
            "2026-01-02T00:00:00Z",
        );

        let formatted = format_storage_manifest(&original);
        let parsed = parse_storage_manifest(&formatted)
            .expect("formatted manifest should parse successfully");

        assert_eq!(parsed, original);
    }

    #[test]
    fn parse_storage_version_and_record_format_keep_unknown_values_explicit() {
        assert_eq!(parse_storage_version("V1".to_owned()), StorageVersion::V1);
        assert_eq!(
            parse_storage_version("VX".to_owned()),
            StorageVersion::Unsupported("VX".to_owned())
        );

        assert_eq!(
            parse_record_format("JsonLinesV1".to_owned()),
            RecordFormat::JsonLinesV1
        );
        assert_eq!(
            parse_record_format("BinaryV99".to_owned()),
            RecordFormat::Unsupported("BinaryV99".to_owned())
        );
    }

    #[test]
    fn validate_storage_manifest_rejects_empty_timestamps() {
        let empty_created = manifest_with("graph--id", " ", "2026-01-02T00:00:00Z");
        let error = validate_storage_manifest(&empty_created)
            .expect_err("empty created_at should be rejected");
        assert!(matches!(
        error,
        GraphStorageError::InvalidManifest { reason } if reason == "created_at must not be empty"
        ));

        let empty_updated = manifest_with("graph--id", "2026-01-01T00:00:00Z", " ");
        let error = validate_storage_manifest(&empty_updated)
            .expect_err("empty updated_at should be rejected");
        assert!(matches!(
        error,
        GraphStorageError::InvalidManifest { reason } if reason == "updated_at must not be empty"
        ));
    }

    #[test]
    fn validate_storage_manifest_rejects_unsupported_version_and_format_and_empty_graph_id() {
        let unsupported_version = StorageManifest {
            storage_version: StorageVersion::Unsupported("V999".to_owned()),
            ..manifest_with("graph--id", "2026-01-01T00:00:00Z", "2026-01-02T00:00:00Z")
        };
        let error = validate_storage_manifest(&unsupported_version)
            .expect_err("unsupported storage version should be rejected");
        assert!(matches!(
        error,
        GraphStorageError::UnsupportedStorageVersion { version } if version == "V999"
        ));

        let unsupported_format = StorageManifest {
            record_format: RecordFormat::Unsupported("BinaryV1".to_owned()),
            ..manifest_with("graph--id", "2026-01-01T00:00:00Z", "2026-01-02T00:00:00Z")
        };
        let error = validate_storage_manifest(&unsupported_format)
            .expect_err("unsupported record format should be rejected");
        assert!(matches!(
        error,
        GraphStorageError::UnsupportedRecordFormat { format } if format == "BinaryV1"
        ));

        let empty_graph_id = manifest_with(" ", "2026-01-01T00:00:00Z", "2026-01-02T00:00:00Z");
        let error = validate_storage_manifest(&empty_graph_id)
            .expect_err("empty graph id should be rejected");
        assert!(matches!(
        error,
        GraphStorageError::InvalidManifest { reason } if reason == "graph_id must not be empty"
        ));
    }
}
