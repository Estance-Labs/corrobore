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

//! A chat model behind the server (issue #390).
//!
//! The viewer's model features (filter in words, fill a column, summarize a
//! branch) need text generation. The provider is any OpenAI-compatible
//! `/chat/completions` endpoint; its key stays on the server, so a browser
//! never holds it, and a policy decides who may spend tokens.

use std::time::Duration;

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    app::AppState,
    error::ApiError,
    handlers::graph_history::{Envelope, ok},
};

/// Messages one request may carry, and characters across them.
pub const MAX_MESSAGES: usize = 64;
pub const MAX_PROMPT_CHARS: usize = 200_000;

#[derive(Clone, PartialEq, Eq)]
pub struct ModelConfig {
    pub endpoint: String,
    pub model: String,
    pub api_key: Option<String>,
    pub max_tokens: u32,
    pub timeout_ms: u64,
}

impl std::fmt::Debug for ModelConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelConfig")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("max_tokens", &self.max_tokens)
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompleteBody {
    pub messages: Vec<Message>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Ask for a JSON object answer.
    #[serde(default)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
pub struct Completion {
    pub text: String,
    pub model: String,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ModelStatus {
    pub configured: bool,
    pub model: Option<String>,
    pub max_tokens: Option<u32>,
}

/// `GET /v1/model`: whether a model is available, for the viewer to show
/// or hide its model features.
pub async fn status(State(state): State<AppState>) -> Json<Envelope<ModelStatus>> {
    let config = state.config.model.as_ref();
    ok(ModelStatus {
        configured: config.is_some(),
        model: config.map(|config| config.model.clone()),
        max_tokens: config.map(|config| config.max_tokens),
    })
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::bad_request("INVALID_MODEL_REQUEST", message)
}

/// `POST /v1/model/complete`.
pub async fn complete(
    State(state): State<AppState>,
    Json(body): Json<CompleteBody>,
) -> Result<Json<Envelope<Completion>>, ApiError> {
    let Some(config) = state.config.model.clone() else {
        return Err(ApiError::not_found(
            "MODEL_NOT_CONFIGURED",
            "this server has no model provider",
        ));
    };
    if body.messages.is_empty() || body.messages.len() > MAX_MESSAGES {
        return Err(invalid(format!(
            "messages must hold 1 to {MAX_MESSAGES} entries"
        )));
    }
    let chars: usize = body
        .messages
        .iter()
        .map(|m| m.content.chars().count())
        .sum();
    if chars > MAX_PROMPT_CHARS {
        return Err(invalid(format!(
            "messages hold more than {MAX_PROMPT_CHARS} characters"
        )));
    }
    if body.messages.last().map(|m| &m.role) != Some(&Role::User) {
        return Err(invalid("the last message must come from the user"));
    }
    if let Some(temperature) = body.temperature
        && !(0.0..=2.0).contains(&temperature)
    {
        return Err(invalid("temperature must be between 0 and 2"));
    }
    let max_tokens = body
        .max_tokens
        .unwrap_or(config.max_tokens)
        .clamp(1, config.max_tokens);
    let mut request = json!({
        "model": config.model,
        "messages": body.messages,
        "max_tokens": max_tokens,
        "stream": false,
    });
    if let Some(temperature) = body.temperature {
        request["temperature"] = json!(temperature);
    }
    if body.json {
        request["response_format"] = json!({"type": "json_object"});
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(config.timeout_ms))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ApiError::internal("MODEL_CLIENT_FAILED", "model client unavailable"))?;
    let mut call = client.post(&config.endpoint).json(&request);
    if let Some(key) = config.api_key.as_deref() {
        call = call.bearer_auth(key);
    }
    let response = call.send().await.map_err(|error| {
        if error.is_timeout() {
            ApiError {
                status: axum::http::StatusCode::GATEWAY_TIMEOUT,
                code: "MODEL_TIMEOUT",
                message: format!("the model did not answer within {} ms", config.timeout_ms),
            }
        } else {
            upstream("the model provider is unreachable")
        }
    })?;
    let status = response.status();
    let answer: Value = response
        .json()
        .await
        .map_err(|_| upstream("the model provider answered something other than JSON"))?;
    if !status.is_success() {
        let detail = answer["error"]["message"]
            .as_str()
            .map(|message| message.chars().take(300).collect::<String>())
            .unwrap_or_default();
        return Err(upstream(
            format!("the model provider answered {status} {detail}").trim_end(),
        ));
    }
    let choice = &answer["choices"][0];
    let text = choice["message"]["content"]
        .as_str()
        .ok_or_else(|| upstream("the model answer has no message content"))?
        .to_owned();
    if body.json && serde_json::from_str::<Value>(&text).map_or(true, |value| !value.is_object()) {
        return Err(upstream("the model did not answer a JSON object"));
    }
    Ok(ok(Completion {
        text,
        model: answer["model"].as_str().unwrap_or(&config.model).to_owned(),
        prompt_tokens: answer["usage"]["prompt_tokens"].as_u64(),
        completion_tokens: answer["usage"]["completion_tokens"].as_u64(),
        finish_reason: choice["finish_reason"].as_str().map(str::to_owned),
    }))
}

fn upstream(message: impl Into<String>) -> ApiError {
    ApiError {
        status: axum::http::StatusCode::BAD_GATEWAY,
        code: "MODEL_PROVIDER_FAILED",
        message: message.into(),
    }
}
