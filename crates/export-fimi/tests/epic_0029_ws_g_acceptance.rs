// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! WS-G acceptance gate for the exporter side of epic #195 (issue #221).
//!
//! Reuse the canonical export contract so the workstream gate cannot silently
//! diverge from it.
//!
//! Epic criteria proven here:
//!
//! - a piece with supported claims exports a high misleadingness assessment
//!   with its mechanism and evidence, in fields distinct from every verdict
//!   (`export::a_supported_claim_and_a_high_misleadingness_assessment_export_distinctly`);
//! - every exported assessment surfaces the records it traces to, and an
//!   annotation citing nothing exports as visibly untraceable
//!   (`export::every_exported_assessment_surfaces_records_the_graph_holds`,
//!   `export::an_annotation_that_cites_nothing_exports_as_visibly_untraceable`);
//! - campaign lineage carries the collections and their coordination evidence,
//!   which never asserts an author
//!   (`export::campaign_lineage_carries_the_collections_and_their_coordination_signals`);
//! - FIMI exports stay byte-identical for graphs with no narrative or campaign
//!   record
//!   (`export::governed_records_without_collections_carry_no_campaign_or_assessment_keys`,
//!   `export::a_collection_that_references_no_exported_record_keeps_the_export_byte_identical`).
//!
//! The core-side criteria are gated by
//! `graph-core/tests/epic_0029_ws_g_acceptance.rs`.
#[path = "campaign_misleadingness_export.rs"]
mod export;
