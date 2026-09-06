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
//! Field-addressed candidate validation and immutable repair attribution.
use crate::{CandidateId, GraphError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
/// JSON value kinds supported by extraction contracts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateValueType {
    /// A JSON string.
    String,
    /// Any JSON number.
    Number,
    /// An integer represented without a fractional component.
    Integer,
    /// A JSON boolean.
    Boolean,
    /// An ordered JSON array.
    Array,
    /// A JSON object.
    Object,
}
/// Explicit constraint categories for candidate extraction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CandidateRule {
    /// Schema requirement: the addressed field exists and is not null.
    Required,
    /// Require compatibility with one JSON value kind.
    Type {
        /// Expected JSON kind.
        expected: CandidateValueType,
    },
    /// Require an array within inclusive length bounds.
    Cardinality {
        /// Minimum item count.
        min: usize,
        /// Optional maximum item count.
        max: Option<usize>,
    },
    /// Require this RFC 3339 timestamp to be at or after another timestamp.
    TemporalOrder {
        /// RFC 6901 pointer to the earlier timestamp.
        after: String,
    },
    /// Require an exact predicate from an explicit vocabulary.
    AllowedPredicates {
        /// Accepted predicate strings.
        allowed: Vec<String>,
    },
    /// Require syntactically valid JSON; also emitted for parse failures.
    JsonDocument,
}
/// Stable rule identity and an RFC 6901 pointer for targeted re-extraction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateConstraint {
    /// Stable constraint identity, unique within one contract.
    pub id: String,
    /// RFC 6901 pointer; the empty pointer addresses the whole document.
    pub field: String,
    /// Typed validation rule and its parameters.
    pub rule: CandidateRule,
}
/// One exact observed failure, including repetition in the preceding repair.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateFailure {
    /// Exact offending pointer, including a malformed temporal counterpart.
    pub field: String,
    /// Exact failing rule, including its field pointer and identity.
    pub constraint: CandidateConstraint,
    /// Unmodified JSON value, or null when the field is absent.
    pub observed: Value,
    /// Distinguishes an absent field from an explicit null.
    pub present: bool,
    /// The same rule also failed on the immediate predecessor.
    pub repeated: bool,
}
/// Deterministic report derived from immutable raw input and retained rules.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateValidation {
    /// No failures under the supplied contract; never an automatic promotion.
    pub valid: bool,
    /// All failing constraints in contract order.
    pub failures: Vec<CandidateFailure>,
}
/// Append-only predecessor link and the failures that prompted re-extraction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateRepair {
    /// Immutable prior candidate version.
    pub predecessor: CandidateId,
    /// Distinct constraint IDs that failed on the predecessor.
    pub caused_by: Vec<String>,
}
fn invalid(message: &str) -> GraphError {
    GraphError::InvalidPropertyValue(message.into())
}
fn valid_pointer(pointer: &str) -> bool {
    if pointer.is_empty() {
        return true;
    }
    if !pointer.starts_with('/') {
        return false;
    }
    let mut chars = pointer.chars();
    while let Some(ch) = chars.next() {
        if ch == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return false;
        }
    }
    true
}
pub(crate) fn validate_contract(constraints: &[CandidateConstraint]) -> Result<(), GraphError> {
    let mut ids = std::collections::HashSet::new();
    for constraint in constraints {
        if constraint.id.trim().is_empty()
            || constraint.id == "$json"
            || !ids.insert(&constraint.id)
        {
            return Err(invalid(
                "constraint IDs must be nonblank, unique, and not reserved",
            ));
        }
        if !valid_pointer(&constraint.field) {
            return Err(invalid("constraint field must be an RFC 6901 pointer"));
        }
        match &constraint.rule {
            CandidateRule::Cardinality {
                min,
                max: Some(max),
            } if min > max => return Err(invalid("invalid cardinality bounds")),
            CandidateRule::TemporalOrder { after } if !valid_pointer(after) => {
                return Err(invalid("invalid temporal counterpart pointer"));
            }
            CandidateRule::AllowedPredicates { allowed }
                if allowed.is_empty() || allowed.iter().any(|p| p.trim().is_empty()) =>
            {
                return Err(invalid("allowed predicates must be nonempty"));
            }
            _ => {}
        }
    }
    Ok(())
}
pub(crate) fn evaluate(raw: &str, constraints: &[CandidateConstraint]) -> CandidateValidation {
    // Unconstrained legacy proposals retain the WS-C 1 import contract.
    if constraints.is_empty() {
        return CandidateValidation {
            valid: true,
            failures: vec![],
        };
    }
    let document: Value = match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(_) => {
            return CandidateValidation {
                valid: false,
                failures: vec![CandidateFailure {
                    constraint: CandidateConstraint {
                        id: "$json".into(),
                        field: String::new(),
                        rule: CandidateRule::JsonDocument,
                    },
                    field: String::new(),
                    observed: Value::String(raw.into()),
                    present: true,
                    repeated: false,
                }],
            };
        }
    };
    let failures = constraints
        .iter()
        .filter_map(|constraint| {
            let mut field = &constraint.field;
            let value = document.pointer(field);
            let passed = match &constraint.rule {
                CandidateRule::Required => value.is_some_and(|value| !value.is_null()),
                CandidateRule::Type { expected } => value.is_some_and(|value| match expected {
                    CandidateValueType::String => value.is_string(),
                    CandidateValueType::Number => value.is_number(),
                    CandidateValueType::Integer => value.is_i64() || value.is_u64(),
                    CandidateValueType::Boolean => value.is_boolean(),
                    CandidateValueType::Array => value.is_array(),
                    CandidateValueType::Object => value.is_object(),
                }),
                CandidateRule::Cardinality { min, max } => {
                    value.and_then(Value::as_array).is_some_and(|values| {
                        values.len() >= *min && max.is_none_or(|max| values.len() <= max)
                    })
                }
                CandidateRule::TemporalOrder { after } => {
                    let end = value
                        .and_then(Value::as_str)
                        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok());
                    let start = document
                        .pointer(after)
                        .and_then(Value::as_str)
                        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok());
                    if start.is_none() {
                        field = after;
                    }
                    matches!((start, end), (Some(start), Some(end)) if start <= end)
                }
                CandidateRule::AllowedPredicates { allowed } => value
                    .and_then(Value::as_str)
                    .is_some_and(|value| allowed.iter().any(|p| p == value)),
                CandidateRule::JsonDocument => true,
            };
            (!passed).then(|| {
                let observed = document.pointer(field);
                CandidateFailure {
                    constraint: constraint.clone(),
                    field: field.clone(),
                    observed: observed.cloned().unwrap_or(Value::Null),
                    present: observed.is_some(),
                    repeated: false,
                }
            })
        })
        .collect::<Vec<_>>();
    CandidateValidation {
        valid: failures.is_empty(),
        failures,
    }
}
