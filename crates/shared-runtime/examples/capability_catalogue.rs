// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Print the capability catalogue as the artifact every adapter reads.
//!
//! `compatibility/capabilities/v1/catalogue.json` is this output, committed. A
//! contract test compares the two, so an adapter in another language cannot
//! drift from the definition without failing the build.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let catalogue = shared_runtime::CapabilityCatalogue::v1();
    catalogue.validate()?;
    println!("{}", serde_json::to_string_pretty(&catalogue)?);
    Ok(())
}
