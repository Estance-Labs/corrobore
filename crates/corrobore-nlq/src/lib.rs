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
//! Natural-language queries as a compile-only adapter over typed contracts
//! (epic #82, item #260; ADR-0020).
//!
//! Crate boundary: this crate defines the versioned `nlq/v1` action envelope,
//! validates untrusted envelopes against a caller-supplied trust boundary,
//! canonicalizes every action through the real Cypher, `INVESTIGATE` and
//! `memory/v1` parsers, and ships a deterministic bilingual template compiler
//! as the baseline. It executes nothing, holds no model, and has no
//! machine-learning dependency. A model, when one exists, is a
//! [`NlqCompiler`] whose output crosses [`validate`] before anything runs.
//!
//! The envelope has no field for trusted runtime context: workspace, session,
//! actor, agent, permissions, request and correlation identity come from the
//! host, and an envelope that carries any of them is refused wherever the key
//! appears. That is what makes a prompt injection inert: the text can say
//! anything, and the runtime decides authorization from context it resolved
//! itself.

pub mod dataset;
pub mod evaluation;
mod template;

use std::collections::BTreeSet;

use cypher_parser::{InvestigationErrorCode, QueryKind, parse_investigation_query, parse_query};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use template::TemplateCompiler;

/// The envelope contract version.
pub const SCHEMA_VERSION: &str = "nlq/v1";

/// Keys that carry trusted runtime context and may never appear in an
/// envelope, at any depth.
pub const TRUSTED_CONTEXT_KEYS: [&str; 9] = [
    "workspace_id",
    "session_id",
    "actor_id",
    "agent_id",
    "permissions",
    "request_id",
    "correlation_id",
    "budget_ref",
    "idempotency_key",
];

/// Request language. French and English are the release languages; other
/// tags travel as given so a model may report them without being trusted.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Language {
    /// French.
    Fr,
    /// English.
    En,
    /// Any other BCP-47 style tag.
    Other(String),
}

impl Language {
    /// The tag as it travels in the envelope.
    #[must_use]
    pub fn tag(&self) -> &str {
        match self {
            Self::Fr => "fr",
            Self::En => "en",
            Self::Other(tag) => tag,
        }
    }

    /// Parse a tag; `fr` and `en` are the release languages.
    #[must_use]
    pub fn from_tag(tag: &str) -> Self {
        match tag.to_ascii_lowercase().as_str() {
            "fr" => Self::Fr,
            "en" => Self::En,
            _ => Self::Other(tag.to_owned()),
        }
    }
}

impl Serialize for Language {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.tag())
    }
}

impl<'de> Deserialize<'de> for Language {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let tag = String::deserialize(deserializer)?;
        if tag.trim().is_empty() {
            return Err(serde::de::Error::custom("language tag must not be empty"));
        }
        Ok(Self::from_tag(&tag))
    }
}

/// Exactly one action per envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// A `memory/v1` operation, typed by name with its input document.
    MemoryOperation {
        /// `remember`, `relate`, `recall`, `update`, `forget`, `consolidate` or `trace`.
        operation: String,
        /// The operation input, validated against the memory contract.
        input: serde_json::Value,
    },
    /// A bounded read-only Cypher query.
    CypherRead {
        /// Query text.
        query: String,
    },
    /// A mutation the caller may execute after inspection; never executed here.
    CypherWriteProposal {
        /// Query text.
        query: String,
    },
    /// A declarative `INVESTIGATE` statement.
    Investigation {
        /// Statement text.
        statement: String,
    },
    /// The request is ambiguous; the compiler asks rather than guesses.
    ClarificationRequired {
        /// The question to put to the user.
        question: String,
        /// Candidate readings, when known.
        #[serde(default)]
        options: Vec<String>,
    },
    /// The compiler declines: no evidence, no permission, or nothing to do.
    Abstain {
        /// Why.
        reason: String,
    },
    /// The request needs a capability Corrobore does not expose.
    Unsupported {
        /// Why.
        reason: String,
    },
}

/// The kind of an [`Action`], without its payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// [`Action::MemoryOperation`].
    MemoryOperation,
    /// [`Action::CypherRead`].
    CypherRead,
    /// [`Action::CypherWriteProposal`].
    CypherWriteProposal,
    /// [`Action::Investigation`].
    Investigation,
    /// [`Action::ClarificationRequired`].
    ClarificationRequired,
    /// [`Action::Abstain`].
    Abstain,
    /// [`Action::Unsupported`].
    Unsupported,
}

impl ActionKind {
    /// Whether the kind ends the exchange rather than naming an action.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::ClarificationRequired | Self::Abstain | Self::Unsupported
        )
    }

    /// The `kind` tag.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::MemoryOperation => "memory_operation",
            Self::CypherRead => "cypher_read",
            Self::CypherWriteProposal => "cypher_write_proposal",
            Self::Investigation => "investigation",
            Self::ClarificationRequired => "clarification_required",
            Self::Abstain => "abstain",
            Self::Unsupported => "unsupported",
        }
    }

    fn allowed_keys(self) -> &'static [&'static str] {
        match self {
            Self::MemoryOperation => &["kind", "operation", "input"],
            Self::CypherRead | Self::CypherWriteProposal => &["kind", "query"],
            Self::Investigation => &["kind", "statement"],
            Self::ClarificationRequired => &["kind", "question", "options"],
            Self::Abstain | Self::Unsupported => &["kind", "reason"],
        }
    }
}

impl Action {
    /// The kind of this action.
    #[must_use]
    pub const fn kind(&self) -> ActionKind {
        match self {
            Self::MemoryOperation { .. } => ActionKind::MemoryOperation,
            Self::CypherRead { .. } => ActionKind::CypherRead,
            Self::CypherWriteProposal { .. } => ActionKind::CypherWriteProposal,
            Self::Investigation { .. } => ActionKind::Investigation,
            Self::ClarificationRequired { .. } => ActionKind::ClarificationRequired,
            Self::Abstain { .. } => ActionKind::Abstain,
            Self::Unsupported { .. } => ActionKind::Unsupported,
        }
    }
}

/// Boundedness metadata a compiler declares for its action.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Boundedness {
    /// Row limit of a read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Traversal depth of a recall.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u32>,
    /// Item cap of a recall.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_items: Option<u32>,
}

/// The `nlq/v1` action envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    /// Always [`SCHEMA_VERSION`].
    pub schema_version: String,
    /// The language of the request the action was compiled from.
    pub language: Language,
    /// Exactly one action.
    pub action: Action,
    /// Declared bounds.
    #[serde(default)]
    pub bounded: Boundedness,
    /// Evidence references the compiler used; each must be caller-supplied.
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    /// Stable reason code (`compiled`, `no_evidence`, `write_not_permitted`, ...).
    pub reason_code: String,
}

impl Envelope {
    /// A versioned envelope with no declared bounds.
    #[must_use]
    pub fn new(language: Language, action: Action, reason_code: impl Into<String>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_owned(),
            language,
            action,
            bounded: Boundedness::default(),
            evidence_refs: Vec::new(),
            reason_code: reason_code.into(),
        }
    }

    /// Declare a row limit.
    #[must_use]
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.bounded.limit = Some(limit);
        self
    }

    /// Record the evidence references the action rests on.
    #[must_use]
    pub fn with_evidence_refs(mut self, refs: impl IntoIterator<Item = String>) -> Self {
        self.evidence_refs = refs.into_iter().collect();
        self
    }
}

/// What the caller allows an envelope to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustBoundary {
    allowed_evidence: BTreeSet<String>,
    allow_writes: bool,
}

impl TrustBoundary {
    /// Evidence references the caller supplied, and whether writes may be
    /// proposed at all.
    pub fn new<I, S>(allowed_evidence: I, allow_writes: bool) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            allowed_evidence: allowed_evidence.into_iter().map(Into::into).collect(),
            allow_writes,
        }
    }

    /// Whether writes may be proposed.
    #[must_use]
    pub const fn allows_writes(&self) -> bool {
        self.allow_writes
    }

    /// Whether a reference was supplied by the caller.
    #[must_use]
    pub fn allows_evidence(&self, reference: &str) -> bool {
        self.allowed_evidence.contains(reference)
    }
}

/// Why an envelope was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionCode {
    /// Not an object, or a required field is missing or mistyped.
    Malformed,
    /// A field the contract does not define.
    UnknownField,
    /// `schema_version` is not [`SCHEMA_VERSION`].
    UnsupportedSchemaVersion,
    /// A trusted-context key appeared somewhere in the envelope.
    TrustedContextSupplied,
    /// An evidence reference the caller did not supply.
    InventedEvidence,
    /// A read without a row limit or an aggregate.
    Unbounded,
    /// A read whose query mutates.
    ReadEmitsWrite,
    /// Cypher the bounded parser rejects.
    UnsupportedCypher,
    /// The action's kind does not match its payload.
    ActionKindMismatch,
    /// A write where the caller allowed none.
    WriteNotAllowed,
    /// An `INVESTIGATE` statement the grammar rejects.
    UnsupportedInvestigation,
    /// A memory input the `memory/v1` contract rejects.
    InvalidMemoryInput,
    /// A memory operation name the contract does not define.
    UnknownMemoryOperation,
}

/// A refused envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct Rejection {
    /// Stable code.
    pub code: RejectionCode,
    /// Human-readable detail; never echoes trusted context values.
    pub message: String,
}

fn reject(code: RejectionCode, message: impl Into<String>) -> Rejection {
    Rejection {
        code,
        message: message.into(),
    }
}

/// An envelope that passed validation, with its canonical form.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedEnvelope {
    envelope: Envelope,
    kind: ActionKind,
    canonical: String,
    writes: bool,
}

impl ValidatedEnvelope {
    /// The action kind.
    #[must_use]
    pub const fn kind(&self) -> ActionKind {
        self.kind
    }

    /// The request language.
    #[must_use]
    pub const fn language(&self) -> &Language {
        &self.envelope.language
    }

    /// The canonical form the real parser produced: normalized Cypher, the
    /// canonical `INVESTIGATE` string, or `memory/v1 <op> <canonical json>`.
    #[must_use]
    pub fn canonical(&self) -> &str {
        &self.canonical
    }

    /// Whether the action changes retained state when executed.
    #[must_use]
    pub const fn writes(&self) -> bool {
        self.writes
    }

    /// Whether the action ends the exchange.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        self.kind.is_terminal()
    }

    /// Evidence references the envelope declared.
    #[must_use]
    pub fn evidence_refs(&self) -> &[String] {
        &self.envelope.evidence_refs
    }

    /// The validated envelope.
    #[must_use]
    pub const fn envelope(&self) -> &Envelope {
        &self.envelope
    }
}

/// Validate an untrusted envelope against the caller's trust boundary and
/// canonicalize its action through the real parsers.
///
/// # Errors
/// [`Rejection`] with a stable [`RejectionCode`].
pub fn validate(
    value: &serde_json::Value,
    boundary: &TrustBoundary,
) -> Result<ValidatedEnvelope, Rejection> {
    let Some(object) = value.as_object() else {
        return Err(reject(
            RejectionCode::Malformed,
            "an envelope is a JSON object",
        ));
    };
    if let Some(key) = find_trusted_context(value) {
        return Err(reject(
            RejectionCode::TrustedContextSupplied,
            format!("`{key}` is trusted runtime context and cannot be supplied by a compiler"),
        ));
    }
    match object
        .get("schema_version")
        .and_then(serde_json::Value::as_str)
    {
        Some(version) if version == SCHEMA_VERSION => {}
        Some(other) => {
            return Err(reject(
                RejectionCode::UnsupportedSchemaVersion,
                format!("schema version `{other}` is not `{SCHEMA_VERSION}`"),
            ));
        }
        None => {
            return Err(reject(
                RejectionCode::Malformed,
                "`schema_version` is required",
            ));
        }
    }
    // The action object is closed per kind; serde's internally tagged enum
    // would accept stray keys, and a stray key is how a second action hides.
    if let Some(action) = object.get("action").and_then(serde_json::Value::as_object) {
        let kind = action
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| reject(RejectionCode::Malformed, "`action.kind` is required"))?;
        let allowed = ACTION_KINDS
            .iter()
            .find(|candidate| candidate.tag() == kind)
            .map(|candidate| candidate.allowed_keys())
            .ok_or_else(|| {
                reject(
                    RejectionCode::Malformed,
                    format!("unknown action kind `{kind}`"),
                )
            })?;
        if let Some(stray) = action.keys().find(|key| !allowed.contains(&key.as_str())) {
            return Err(reject(
                RejectionCode::UnknownField,
                format!("`action.{stray}` is not a field of a `{kind}` action"),
            ));
        }
    }
    let envelope: Envelope = serde_json::from_value(value.clone()).map_err(|error| {
        let message = error.to_string();
        if message.contains("unknown field") {
            reject(RejectionCode::UnknownField, message)
        } else {
            reject(RejectionCode::Malformed, message)
        }
    })?;
    for reference in &envelope.evidence_refs {
        if !boundary.allows_evidence(reference) {
            return Err(reject(
                RejectionCode::InventedEvidence,
                format!("evidence reference `{reference}` was not supplied by the caller"),
            ));
        }
    }
    let kind = envelope.action.kind();
    let (canonical, writes) = match &envelope.action {
        Action::CypherRead { query } => {
            let ast = parse_query(query)
                .map_err(|error| reject(RejectionCode::UnsupportedCypher, error.message))?;
            if ast.kind != QueryKind::Read {
                return Err(reject(
                    RejectionCode::ReadEmitsWrite,
                    "a `cypher_read` action carries a mutation",
                ));
            }
            let bounded = ast
                .query
                .as_ref()
                .and_then(|query| query.return_clause.as_ref())
                .is_some_and(|return_clause| {
                    return_clause.limit.is_some()
                        || (!return_clause.items.is_empty()
                            && return_clause.items.iter().all(|item| {
                                !matches!(
                                    item,
                                    cypher_parser::ProjectionItem::Variable(_)
                                        | cypher_parser::ProjectionItem::Property(_)
                                )
                            }))
                });
            if !bounded {
                return Err(reject(
                    RejectionCode::Unbounded,
                    "a `cypher_read` needs a LIMIT or an aggregate-only projection",
                ));
            }
            (ast.normalized_query, false)
        }
        Action::CypherWriteProposal { query } => {
            let ast = parse_query(query)
                .map_err(|error| reject(RejectionCode::UnsupportedCypher, error.message))?;
            if ast.kind == QueryKind::Read {
                return Err(reject(
                    RejectionCode::ActionKindMismatch,
                    "a `cypher_write_proposal` carries a read; use `cypher_read`",
                ));
            }
            if !boundary.allows_writes() {
                return Err(reject(
                    RejectionCode::WriteNotAllowed,
                    "the caller did not allow writes to be proposed",
                ));
            }
            (ast.normalized_query, true)
        }
        Action::Investigation { statement } => {
            // Every grammar refusal is one code for the caller: which clause
            // failed is in the message, and none of them is recoverable here.
            let query = parse_investigation_query(statement).map_err(|error| {
                let _: InvestigationErrorCode = error.code;
                reject(RejectionCode::UnsupportedInvestigation, error.message)
            })?;
            (query.to_canonical_string(), false)
        }
        Action::MemoryOperation { operation, input } => {
            let (canonical, writes, sources) = memory_canonical(operation, input)?;
            if writes && !boundary.allows_writes() {
                return Err(reject(
                    RejectionCode::WriteNotAllowed,
                    "the caller did not allow writes to be proposed",
                ));
            }
            for source in sources {
                if !boundary.allows_evidence(&source) {
                    return Err(reject(
                        RejectionCode::InventedEvidence,
                        format!("provenance source `{source}` was not supplied by the caller"),
                    ));
                }
            }
            (canonical, writes)
        }
        Action::ClarificationRequired { question, .. } => {
            (format!("clarification_required: {question}"), false)
        }
        Action::Abstain { reason } => (format!("abstain: {reason}"), false),
        Action::Unsupported { reason } => (format!("unsupported: {reason}"), false),
    };
    Ok(ValidatedEnvelope {
        envelope,
        kind,
        canonical,
        writes,
    })
}

const ACTION_KINDS: [ActionKind; 7] = [
    ActionKind::MemoryOperation,
    ActionKind::CypherRead,
    ActionKind::CypherWriteProposal,
    ActionKind::Investigation,
    ActionKind::ClarificationRequired,
    ActionKind::Abstain,
    ActionKind::Unsupported,
];

fn find_trusted_context(value: &serde_json::Value) -> Option<&str> {
    match value {
        serde_json::Value::Object(entries) => {
            for (key, nested) in entries {
                if let Some(trusted) = TRUSTED_CONTEXT_KEYS.iter().find(|trusted| **trusted == key)
                {
                    return Some(trusted);
                }
                if let Some(found) = find_trusted_context(nested) {
                    return Some(found);
                }
            }
            None
        }
        serde_json::Value::Array(items) => items.iter().find_map(find_trusted_context),
        _ => None,
    }
}

/// Canonicalize a memory operation through the `memory/v1` request types.
///
/// Returns the canonical text, whether the operation writes, and the
/// provenance sources it cites.
///
/// # Errors
/// [`Rejection`] for an unknown operation or an input the contract rejects.
pub fn memory_canonical(
    operation: &str,
    input: &serde_json::Value,
) -> Result<(String, bool, Vec<String>), Rejection> {
    use corrobore_engine::{
        ConsolidateRequest, ForgetRequest, MemoryUpdateRequest, RecallRequest, RelateRequest,
        RememberRequest, TraceRequest,
    };
    fn typed<T: serde::de::DeserializeOwned + Serialize>(
        operation: &str,
        input: &serde_json::Value,
    ) -> Result<String, Rejection> {
        let request: T = serde_json::from_value(input.clone()).map_err(|error| {
            reject(
                RejectionCode::InvalidMemoryInput,
                format!("`{operation}` input does not satisfy memory/v1: {error}"),
            )
        })?;
        let canonical = serde_json::to_string(&request)
            .map_err(|error| reject(RejectionCode::InvalidMemoryInput, error.to_string()))?;
        Ok(format!("memory/v1 {operation} {canonical}"))
    }
    let sources = |paths: &[&str]| -> Vec<String> {
        paths
            .iter()
            .filter_map(|path| input.get(path))
            .filter_map(serde_json::Value::as_array)
            .flatten()
            .filter_map(|reference| reference.get("source_id"))
            .filter_map(serde_json::Value::as_str)
            .map(str::to_owned)
            .collect()
    };
    Ok(match operation {
        "remember" => (
            typed::<RememberRequest>(operation, input)?,
            true,
            sources(&["provenance"]),
        ),
        "relate" => (
            typed::<RelateRequest>(operation, input)?,
            true,
            sources(&["provenance"]),
        ),
        "recall" => (typed::<RecallRequest>(operation, input)?, false, Vec::new()),
        "update" => {
            let canonical = typed::<MemoryUpdateRequest>(operation, input)?;
            let sources = input
                .get("patch")
                .and_then(|patch| patch.get("add_provenance"))
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|reference| reference.get("source_id"))
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect();
            (canonical, true, sources)
        }
        "forget" => (typed::<ForgetRequest>(operation, input)?, true, Vec::new()),
        "consolidate" => (
            typed::<ConsolidateRequest>(operation, input)?,
            true,
            Vec::new(),
        ),
        "trace" => (typed::<TraceRequest>(operation, input)?, false, Vec::new()),
        other => {
            return Err(reject(
                RejectionCode::UnknownMemoryOperation,
                format!("`{other}` is not a memory/v1 operation"),
            ));
        }
    })
}

/// A natural-language request with the context a compiler may use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NlqRequest {
    text: String,
    language: Option<Language>,
    evidence_refs: Vec<String>,
    allow_writes: bool,
}

impl NlqRequest {
    /// A read-only request with no evidence and no declared language.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            language: None,
            evidence_refs: Vec::new(),
            allow_writes: false,
        }
    }

    /// Declare the language instead of letting the compiler detect it.
    #[must_use]
    pub fn with_language(mut self, language: Language) -> Self {
        self.language = Some(language);
        self
    }

    /// Evidence references the caller supplies; only these may be cited.
    #[must_use]
    pub fn with_evidence_refs(mut self, refs: impl IntoIterator<Item = String>) -> Self {
        self.evidence_refs = refs.into_iter().collect();
        self
    }

    /// Whether the task allows writes to be proposed.
    #[must_use]
    pub const fn allow_writes(mut self, allow: bool) -> Self {
        self.allow_writes = allow;
        self
    }

    /// The request text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The declared language, if any.
    #[must_use]
    pub const fn language(&self) -> Option<&Language> {
        self.language.as_ref()
    }

    /// Caller-supplied evidence references.
    #[must_use]
    pub fn evidence_refs(&self) -> &[String] {
        &self.evidence_refs
    }

    /// Whether writes may be proposed.
    #[must_use]
    pub const fn writes_allowed(&self) -> bool {
        self.allow_writes
    }

    /// The trust boundary this request implies.
    #[must_use]
    pub fn boundary(&self) -> TrustBoundary {
        TrustBoundary::new(self.evidence_refs.iter().cloned(), self.allow_writes)
    }
}

/// A compiler from natural language to an envelope. The output is untrusted
/// until [`validate`] accepts it; implementations may be a template engine, a
/// local model or a remote one, and none of them executes anything.
pub trait NlqCompiler {
    /// A stable name recorded in evaluation reports.
    fn name(&self) -> &'static str;

    /// Compile one request.
    fn compile(&self, request: &NlqRequest) -> Envelope;
}
