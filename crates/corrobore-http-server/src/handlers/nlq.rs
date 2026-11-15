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

//! `POST /v1/nlq/compile` (issue #353): compile a French or English request
//! into one validated `nlq/v1` action, without executing it.
//!
//! The deterministic `TemplateCompiler` compiles; the envelope is then
//! validated against the trust boundary the request sets (allowed evidence,
//! whether writes are allowed) and canonicalized through the real parsers.
//! The caller decides whether and where to run the canonical action, through
//! the routes it already uses.

use axum::{Json, extract::State};
use corrobore_nlq::{Language, NlqCompiler, NlqRequest, TemplateCompiler, validate};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    app::AppState,
    error::ApiError,
    handlers::graph_history::{Envelope, ok},
};

/// Longest request text, in characters.
pub const MAX_TEXT_CHARS: usize = 4_000;
/// Most evidence references one request may allow.
pub const MAX_EVIDENCE_REFS: usize = 100;

#[derive(Debug, Deserialize)]
pub struct CompileBody {
    pub text: String,
    pub language: Option<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub allow_writes: bool,
}

#[derive(Debug, Serialize)]
pub struct CompileResult {
    /// Action kind: `cypher_read`, `cypher_write_proposal`, `investigation`,
    /// `memory_operation`, `clarification_required`, `abstain` or `unsupported`.
    pub kind: &'static str,
    /// Language the request was compiled in.
    pub language: String,
    /// Canonical form of the validated action.
    pub canonical: String,
    /// Whether running the action would write.
    pub writes: bool,
    /// Compiler that produced the envelope.
    pub compiler: &'static str,
    /// The validated `nlq/v1` envelope.
    pub envelope: Value,
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::bad_request("INVALID_NLQ_REQUEST", message)
}

/// `POST /v1/nlq/compile`.
pub async fn compile(
    State(_state): State<AppState>,
    Json(body): Json<CompileBody>,
) -> Result<Json<Envelope<CompileResult>>, ApiError> {
    let text = body.text.trim();
    if text.is_empty() || body.text.chars().count() > MAX_TEXT_CHARS {
        return Err(invalid(format!(
            "text must hold 1 to {MAX_TEXT_CHARS} characters"
        )));
    }
    if body.evidence_refs.len() > MAX_EVIDENCE_REFS
        || body
            .evidence_refs
            .iter()
            .any(|reference| reference.trim().is_empty())
    {
        return Err(invalid(format!(
            "evidence_refs must hold at most {MAX_EVIDENCE_REFS} non-blank references"
        )));
    }
    let mut request = NlqRequest::new(text)
        .with_evidence_refs(body.evidence_refs)
        .allow_writes(body.allow_writes);
    if let Some(tag) = body.language.as_deref() {
        match Language::from_tag(tag.trim()) {
            Language::Other(other) => {
                return Err(invalid(format!("language must be fr or en, not {other:?}")));
            }
            language => request = request.with_language(language),
        }
    }
    let compiler = TemplateCompiler;
    let envelope = compiler.compile(&request);
    let value = serde_json::to_value(&envelope)
        .map_err(|error| ApiError::internal("NLQ_ENCODING_FAILED", error.to_string()))?;
    let validated = validate(&value, &request.boundary()).map_err(|rejection| {
        let code = serde_json::to_value(rejection.code)
            .ok()
            .and_then(|code| code.as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("{:?}", rejection.code));
        ApiError::unprocessable("NLQ_REJECTED", format!("{code}: {}", rejection.message))
    })?;
    Ok(ok(CompileResult {
        kind: validated.kind().tag(),
        language: validated.language().tag().to_owned(),
        canonical: validated.canonical().to_owned(),
        writes: validated.writes(),
        compiler: compiler.name(),
        envelope: value,
    }))
}
