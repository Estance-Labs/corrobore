// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Compatibility contract for the additive `medical` and `research` domain
//! names introduced at ABI minor 1. The existing CTI, FIMI, and crisis
//! providers must keep loading unchanged, and unknown names must still fail
//! closed.

use domain_provider_abi::{CORROBORE_DOMAIN_PROVIDER_ABI_MAJOR_V1, DomainName, ProviderMetadata};
use serde_json::json;

fn metadata_json(domain: &str) -> serde_json::Value {
    json!({
        "schema_version": "1",
        "provider_id": format!("fr.estance.corrobore.domain.{domain}"),
        "provider_version": "0.1.0",
        "domain": domain,
        "thread_safe": true,
        "max_concurrency": 1,
        "max_request_bytes": 1048576,
        "max_response_bytes": 1048576,
        "capabilities": [{"name": "node.validate", "version": "1"}]
    })
}

#[test]
fn existing_domain_providers_load_unchanged_after_minor_bump() {
    // The bump is additive: the major stays at 1, so a host built against an
    // earlier minor keeps loading these providers. The exact minor value is
    // asserted in `abi_contract`.
    assert_eq!(CORROBORE_DOMAIN_PROVIDER_ABI_MAJOR_V1, 1);

    for (name, expected) in [
        ("cti", DomainName::Cti),
        ("fimi", DomainName::Fimi),
        ("crisis", DomainName::Crisis),
    ] {
        let metadata: ProviderMetadata = serde_json::from_value(metadata_json(name))
            .expect("existing domain metadata must still deserialize");
        assert_eq!(metadata.domain, expected);
        assert_eq!(metadata.domain.as_str(), name);
    }
}

#[test]
fn new_domain_names_deserialize_and_map_to_stable_strings() {
    for (name, expected) in [
        ("medical", DomainName::Medical),
        ("research", DomainName::Research),
    ] {
        let metadata: ProviderMetadata = serde_json::from_value(metadata_json(name))
            .expect("new domain metadata must deserialize");
        assert_eq!(metadata.domain, expected);
        assert_eq!(metadata.domain.as_str(), name);

        // Round-trips through serialization without drift.
        let encoded = serde_json::to_value(metadata.domain).expect("domain must serialize");
        assert_eq!(encoded, json!(name));
    }
}

#[test]
fn unknown_domain_names_still_fail_closed() {
    let error = serde_json::from_value::<ProviderMetadata>(metadata_json("astrology"))
        .expect_err("unknown domains must fail closed");
    assert!(error.to_string().contains("unknown variant"));
}
