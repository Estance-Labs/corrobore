// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! OpenCTI compatibility mapping outside generic graph-core domain rules.

#![warn(missing_docs)]

mod mapping;
mod projection;

pub use mapping::{
    AccessMetadata, Identifier, IdentifierKind, MappedObject, MappedRecord, MappedRelationship,
    MappingError, MappingVersion, OpenCtiAdapter, OpenCtiTimestamps, Provenance, RecordFamily,
    RecordKind, RecordRef, Reference,
};
pub use projection::{
    IdentifierProjection, IdentifierTransaction, MergeSource, ProjectionApply, ProjectionError,
    ProjectionRecord,
};
