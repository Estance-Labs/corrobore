// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::{collections::HashSet, path::Path};

use domain_provider_abi::{CapabilityDeclaration, DomainName, SCHEMA_V1};
use serde::Deserialize;
use thiserror::Error;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DomainProviderManifest {
    schema_version: String,
    providers: Vec<DomainProviderManifestEntry>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DomainProviderManifestEntry {
    pub domain: DomainName,
    pub library: String,
    pub sha256: String,
    pub required: bool,
    pub capabilities: Vec<CapabilityDeclaration>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum ManifestError {
    #[error("invalid provider manifest JSON: {0}")]
    InvalidJson(String),
    #[error("unsupported provider manifest schema version: {0}")]
    UnsupportedSchema(String),
    #[error("duplicate provider domain: {0}")]
    DuplicateDomain(String),
    #[error("invalid library path for domain {domain}: {path}")]
    InvalidLibraryPath { domain: String, path: String },
    #[error("invalid SHA-256 for domain: {domain}")]
    InvalidSha256 { domain: String },
}

impl DomainProviderManifest {
    pub(crate) fn from_json(raw: &str) -> Result<Self, ManifestError> {
        let manifest: Self = serde_json::from_str(raw)
            .map_err(|error| ManifestError::InvalidJson(error.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub(crate) fn providers(&self) -> &[DomainProviderManifestEntry] {
        &self.providers
    }

    fn validate(&self) -> Result<(), ManifestError> {
        if self.schema_version != SCHEMA_V1 {
            return Err(ManifestError::UnsupportedSchema(
                self.schema_version.clone(),
            ));
        }

        let mut domains = HashSet::new();
        for provider in &self.providers {
            let domain = provider.domain.as_str();
            if !domains.insert(domain) {
                return Err(ManifestError::DuplicateDomain(domain.to_owned()));
            }

            let path = Path::new(&provider.library);
            if path.is_absolute()
                || path
                    .components()
                    .any(|component| !matches!(component, std::path::Component::Normal(_)))
            {
                return Err(ManifestError::InvalidLibraryPath {
                    domain: domain.to_owned(),
                    path: provider.library.clone(),
                });
            }

            if provider.sha256.len() != 64
                || !provider
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(ManifestError::InvalidSha256 {
                    domain: domain.to_owned(),
                });
            }
        }

        Ok(())
    }
}
