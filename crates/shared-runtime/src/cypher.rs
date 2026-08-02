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
use crate::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Cypher request mode.
pub enum CypherRequestMode {
    /// Read only.
    ReadOnly,
    /// Mutation.
    Mutation,
    /// Explain.
    Explain,
    /// Validate only.
    ValidateOnly,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// A typed parameter value carried across the runtime boundary.
///
/// Types are preserved end to end so a bound value reaches the executor as the
/// scalar the caller supplied. Flattening everything to text is what previously
/// made `LIMIT $n` and numeric comparisons silently return the wrong rows.
pub enum CypherValue {
    /// UTF-8 text.
    String(String),
    /// Signed integer.
    Integer(i64),
    /// Finite decimal encoded losslessly as source text.
    Float(String),
    /// Boolean.
    Boolean(bool),
    /// Explicit null.
    Null,
    /// Bounded homogeneous scalar list.
    List(Vec<CypherValue>),
}

impl CypherValue {
    /// Names the type for diagnostics and audit records.
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::String(_) => "string",
            Self::Integer(_) => "integer",
            Self::Float(_) => "float",
            Self::Boolean(_) => "boolean",
            Self::Null => "null",
            Self::List(_) => "list",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
/// Cypher parameters.
pub struct CypherParameters {
    values: HashMap<String, CypherValue>,
}

impl CypherParameters {
    /// Creates parameters whose values are all text.
    ///
    /// Retained for callers that genuinely only bind strings; prefer
    /// [`CypherParameters::typed`] so numeric and boolean values keep their type.
    pub fn new(values: HashMap<String, String>) -> Self {
        Self {
            values: values
                .into_iter()
                .map(|(name, value)| (name, CypherValue::String(value)))
                .collect(),
        }
    }

    /// Creates parameters from already-typed values.
    pub fn typed(values: HashMap<String, CypherValue>) -> Self {
        Self { values }
    }

    /// Values.
    pub fn values(&self) -> &HashMap<String, CypherValue> {
        &self.values
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Cypher budget ref.
pub struct CypherBudgetRef {
    value: String,
}

impl CypherBudgetRef {
    /// Creates a new instance.
    pub fn new(value: impl Into<String>) -> Result<Self, RuntimeError> {
        let value = value.into();

        if value.trim().is_empty() {
            return Err(RuntimeError::MalformedCypherRequest("budget_ref"));
        }

        Ok(Self { value })
    }

    /// Returns the value as str.
    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Cypher request.
pub struct CypherRequest {
    /// Query text.
    pub query_text: String,
    /// Parameters.
    pub parameters: CypherParameters,
    /// Request mode.
    pub mode: CypherRequestMode,
    /// Workspace id.
    pub workspace_id: WorkspaceId,
    /// Session id.
    pub session_id: SessionId,
    /// Budget ref.
    pub budget_ref: CypherBudgetRef,
}

impl CypherRequest {
    // Cypher is a controlled runtime gateway, not a raw escape hatch.
    // Every request is explicitly scoped with workspace/session/budget context
    // so agents cannot bypass runtime safety contracts.
    /// Creates a new instance.
    pub fn new(
        query_text: impl Into<String>,
        parameters: CypherParameters,
        mode: CypherRequestMode,
        workspace_id: WorkspaceId,
        session_id: SessionId,
        budget_ref: CypherBudgetRef,
    ) -> Result<Self, RuntimeError> {
        let query_text = query_text.into();

        if query_text.trim().is_empty() {
            return Err(RuntimeError::MalformedCypherRequest("query_text"));
        }

        Ok(Self {
            query_text,
            parameters,
            mode,
            workspace_id,
            session_id,
            budget_ref,
        })
    }

    /// Creates the read only request.
    pub fn build_read_only_request(
        query_text: impl Into<String>,
        parameters: CypherParameters,
        workspace_id: WorkspaceId,
        session_id: SessionId,
        budget_ref: CypherBudgetRef,
    ) -> Result<Self, RuntimeError> {
        Self::new(
            query_text,
            parameters,
            CypherRequestMode::ReadOnly,
            workspace_id,
            session_id,
            budget_ref,
        )
    }

    /// Creates the mutation request.
    pub fn build_mutation_request(
        query_text: impl Into<String>,
        parameters: CypherParameters,
        workspace_id: WorkspaceId,
        session_id: SessionId,
        budget_ref: CypherBudgetRef,
    ) -> Result<Self, RuntimeError> {
        Self::new(
            query_text,
            parameters,
            CypherRequestMode::Mutation,
            workspace_id,
            session_id,
            budget_ref,
        )
    }

    /// Creates the validate only request.
    pub fn build_validate_only_request(
        query_text: impl Into<String>,
        parameters: CypherParameters,
        workspace_id: WorkspaceId,
        session_id: SessionId,
        budget_ref: CypherBudgetRef,
    ) -> Result<Self, RuntimeError> {
        Self::new(
            query_text,
            parameters,
            CypherRequestMode::ValidateOnly,
            workspace_id,
            session_id,
            budget_ref,
        )
    }

    /// Validates the for gateway execution.
    pub fn validate_for_gateway_execution(&self) -> Result<(), RuntimeError> {
        if self.query_text.trim().is_empty() {
            return Err(RuntimeError::MalformedCypherRequest("query_text"));
        }

        if self.budget_ref.as_str().trim().is_empty() {
            return Err(RuntimeError::MalformedCypherRequest("budget_ref"));
        }

        match self.mode {
            CypherRequestMode::Explain => Err(RuntimeError::UnsupportedCypherRequestMode(
                self.mode.clone(),
            )),
            CypherRequestMode::ReadOnly
            | CypherRequestMode::Mutation
            | CypherRequestMode::ValidateOnly => Ok(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Cypher response status.
pub enum CypherResponseStatus {
    /// Success.
    Success,
    /// Validation failed.
    ValidationFailed,
    /// Rejected.
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Cypher record.
pub struct CypherRecord {
    /// Fields.
    pub fields: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Cypher mutation summary.
pub struct CypherMutationSummary {
    /// Rows matched before mutation execution.
    #[serde(default)]
    pub matched_rows: u64,
    /// Created nodes.
    pub created_nodes: u64,
    /// Updated nodes.
    pub updated_nodes: u64,
    /// Deleted nodes.
    pub deleted_nodes: u64,
    /// Created relationships.
    pub created_relationships: u64,
    /// Updated relationships.
    #[serde(default)]
    pub updated_relationships: u64,
    /// Deleted relationships.
    pub deleted_relationships: u64,
    /// Properties set.
    pub properties_set: u64,
    /// Native metadata fields changed.
    #[serde(default)]
    pub native_fields_changed: u64,
    /// Generic property fields changed.
    #[serde(default)]
    pub property_fields_changed: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Cypher response data.
pub enum CypherResponseData {
    /// Records.
    Records(Vec<CypherRecord>),
    /// Mutation summary.
    MutationSummary(CypherMutationSummary),
    /// Empty.
    Empty,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Cypher validation error.
pub struct CypherValidationError {
    /// Code.
    pub code: String,
    /// Message.
    pub message: String,
    /// Field.
    pub field: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Cypher budget usage.
pub struct CypherBudgetUsage {
    /// Budget ref.
    pub budget_ref: CypherBudgetRef,
    /// Consumed units.
    pub consumed_units: u64,
    /// Remaining units.
    pub remaining_units: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Cypher audit reference.
pub struct CypherAuditReference {
    /// Transaction id.
    pub transaction_id: Option<TransactionId>,
    /// Request id.
    pub request_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Cypher fix hint.
pub struct CypherFixHint {
    /// Code.
    pub code: String,
    /// Message.
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Cypher response.
pub struct CypherResponse {
    /// Status.
    pub status: CypherResponseStatus,
    /// Data.
    pub data: CypherResponseData,
    /// Warnings.
    pub warnings: Vec<String>,
    /// Validation errors.
    pub validation_errors: Vec<CypherValidationError>,
    /// Budget usage.
    pub budget_usage: Option<CypherBudgetUsage>,
    /// Audit references.
    pub audit_references: Vec<CypherAuditReference>,
    /// Fix hints.
    pub fix_hints: Vec<CypherFixHint>,
}
