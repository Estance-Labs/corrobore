// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

pub(crate) mod manifest;
pub(crate) mod registry;

#[cfg(test)]
mod tests {
    use super::manifest::{DomainProviderManifest, ManifestError};

    const VALID_MANIFEST: &str = r#"{
		"schema_version": "1",
		"providers": [
			{
				"domain": "cti",
				"library": "libdomain_cti.dylib",
				"sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
				"required": true,
				"capabilities": [{"name": "node.validate", "version": "1"}]
			},
			{
				"domain": "fimi",
				"library": "libdomain_fimi.dylib",
				"sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
				"required": false,
				"capabilities": [{"name": "node.validate", "version": "1"}]
			}
		]
	}"#;

    #[test]
    fn provider_manifest_contract_parses_strict_valid_entries() {
        let manifest = DomainProviderManifest::from_json(VALID_MANIFEST)
            .expect("valid provider manifest should parse");

        assert_eq!(manifest.providers().len(), 2);
        assert_eq!(manifest.providers()[0].domain.as_str(), "cti");
        assert!(manifest.providers()[0].required);
        assert!(!manifest.providers()[1].required);
    }

    #[test]
    fn provider_manifest_contract_rejects_duplicate_domains() {
        let duplicate = VALID_MANIFEST.replace("\"domain\": \"fimi\"", "\"domain\": \"cti\"");

        let error = DomainProviderManifest::from_json(&duplicate)
            .expect_err("duplicate domains must fail closed");

        assert_eq!(error, ManifestError::DuplicateDomain("cti".to_owned()));
    }

    #[test]
    fn provider_manifest_contract_rejects_unsafe_paths_and_hashes() {
        let absolute = VALID_MANIFEST.replace("libdomain_cti.dylib", "/tmp/libdomain_cti.dylib");
        assert!(matches!(
            DomainProviderManifest::from_json(&absolute),
            Err(ManifestError::InvalidLibraryPath { .. })
        ));

        let traversal = VALID_MANIFEST.replace("libdomain_cti.dylib", "../libdomain_cti.dylib");
        assert!(matches!(
            DomainProviderManifest::from_json(&traversal),
            Err(ManifestError::InvalidLibraryPath { .. })
        ));

        let invalid_hash = VALID_MANIFEST.replacen(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "ABC123",
            1,
        );
        assert!(matches!(
            DomainProviderManifest::from_json(&invalid_hash),
            Err(ManifestError::InvalidSha256 { .. })
        ));
    }
}
