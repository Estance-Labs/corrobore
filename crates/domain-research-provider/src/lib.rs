// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![warn(missing_docs)]

//! Dynamic-library entry point for the RESEARCH domain pack.
//!
//! Every pack exports the ABI v1 entry point under the same symbol,
//! `corrobore_domain_provider_get_api_v1`, because the host resolves it per
//! loaded library. That symbol therefore belongs to the `cdylib` artifact and
//! must not appear in an `rlib`: two packs linked into one binary would
//! otherwise be a duplicate-symbol link error.
//!
//! The pack's logic, types, and provider table live in `domain-research`, which
//! stays a pure `rlib` and can be linked alongside other packs.

use domain_provider_abi::DomainProviderApiV1;
use domain_research::research_provider_api_v1;

/// Returns the stable ABI v1 function table consumed by the Corrobore host.
#[unsafe(no_mangle)]
pub extern "C" fn corrobore_domain_provider_get_api_v1() -> *const DomainProviderApiV1 {
    research_provider_api_v1()
}
