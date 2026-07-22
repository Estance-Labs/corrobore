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
use crate::{GraphError, TransactionId};

/// Export strictness mode for deterministic exporter behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportMode {
    /// Strict.
    Strict,
    /// Permissive.
    Permissive,
}

/// Export profile selector for deterministic MVP exporters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportProfile {
    /// Stix mvp.
    StixMvp,
    /// Fimi json mvp.
    FimiJsonMvp,
}

/// Validation report metadata attached to an export event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationReportRef {
    validation_report_id: String,
    ruleset_version: Option<String>,
}

impl ValidationReportRef {
    ///
    /// validate and build a typed validation-report reference.
    pub fn new(
        validation_report_id: impl Into<String>,
        ruleset_version: Option<String>,
    ) -> Result<Self, GraphError> {
        let validation_report_id = validation_report_id.into();

        if validation_report_id.trim().is_empty() {
            return Err(GraphError::InvalidExportMetadataField(
                "validation_report_id".to_owned(),
            ));
        }

        Ok(Self {
            validation_report_id,
            ruleset_version,
        })
    }

    /// Return the stable validation-report identifier.
    pub fn validation_report_id(&self) -> &str {
        self.validation_report_id.as_str()
    }

    /// Return the optional validation ruleset version.
    pub fn ruleset_version(&self) -> Option<&str> {
        self.ruleset_version.as_deref()
    }
}

/// Deterministic metadata identity for export reproducibility.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportMetadata {
    snapshot_id: String,
    transaction_id: TransactionId,
    exporter_version: String,
    profile: ExportProfile,
    mode: ExportMode,
    validation_report: Option<ValidationReportRef>,
}

impl ExportMetadata {
    ///
    /// validate and build deterministic export metadata identity.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        snapshot_id: impl Into<String>,
        transaction_id: TransactionId,
        exporter_version: impl Into<String>,
        profile: ExportProfile,
        mode: ExportMode,
        validation_report: Option<ValidationReportRef>,
    ) -> Result<Self, GraphError> {
        let snapshot_id = snapshot_id.into();
        if snapshot_id.trim().is_empty() {
            return Err(GraphError::InvalidExportMetadataField(
                "snapshot_id".to_owned(),
            ));
        }

        let exporter_version = exporter_version.into();
        if exporter_version.trim().is_empty() {
            return Err(GraphError::InvalidExportMetadataField(
                "exporter_version".to_owned(),
            ));
        }

        Ok(Self {
            snapshot_id,
            transaction_id,
            exporter_version,
            profile,
            mode,
            validation_report,
        })
    }

    /// Return the snapshot ID captured by this export metadata.
    pub fn snapshot_id(&self) -> &str {
        self.snapshot_id.as_str()
    }

    /// Return the transaction ID captured by this export metadata.
    pub fn transaction_id(&self) -> &TransactionId {
        &self.transaction_id
    }

    /// Return the exporter version used to build the payload.
    pub fn exporter_version(&self) -> &str {
        self.exporter_version.as_str()
    }

    /// Return the export profile selected for this payload.
    pub fn profile(&self) -> &ExportProfile {
        &self.profile
    }

    /// Return the export mode selected for this payload.
    pub fn mode(&self) -> ExportMode {
        self.mode
    }

    /// Return the optional validation report reference.
    pub fn validation_report(&self) -> Option<&ValidationReportRef> {
        self.validation_report.as_ref()
    }

    ///
    /// build deterministic key from identity tuple.
    pub fn determinism_key(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}",
            self.snapshot_id,
            self.transaction_id.as_str(),
            self.exporter_version,
            self.profile.as_str(),
            self.mode.as_str()
        )
    }
}

impl ExportMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Permissive => "permissive",
        }
    }
}

impl ExportProfile {
    fn as_str(&self) -> &'static str {
        match self {
            Self::StixMvp => "stix-mvp",
            Self::FimiJsonMvp => "fimi-json-mvp",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transaction_id(value: &str) -> TransactionId {
        TransactionId::new(value).expect("test transaction ID should be valid")
    }

    #[test]
    fn validation_report_ref_exposes_optional_ruleset_version() {
        let with_ruleset = ValidationReportRef::new(
            "validation-report--with-ruleset",
            Some("ruleset-v1".to_owned()),
        )
        .expect("validation report ref should be valid");
        let without_ruleset = ValidationReportRef::new("validation-report--without-ruleset", None)
            .expect("validation report ref should be valid");

        assert_eq!(
            with_ruleset.validation_report_id(),
            "validation-report--with-ruleset"
        );
        assert_eq!(with_ruleset.ruleset_version(), Some("ruleset-v1"));
        assert_eq!(without_ruleset.ruleset_version(), None);
    }

    #[test]
    fn export_metadata_without_validation_report_returns_none() {
        let metadata = ExportMetadata::new(
            "snapshot--unit-none-report",
            transaction_id("transaction--unit-none-report"),
            "exporter-v1",
            ExportProfile::FimiJsonMvp,
            ExportMode::Permissive,
            None,
        )
        .expect("export metadata should be valid");

        assert_eq!(metadata.validation_report(), None);
        assert_eq!(
            metadata.determinism_key(),
            "snapshot--unit-none-report|transaction--unit-none-report|exporter-v1|fimi-json-mvp|permissive"
        );
    }
}
