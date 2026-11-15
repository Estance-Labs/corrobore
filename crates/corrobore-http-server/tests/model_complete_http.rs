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

//! Issue #390: `POST /v1/model/complete` relays to an OpenAI-compatible
//! chat endpoint whose key stays on the server.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod graph_api_support;

use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, Method, StatusCode},
    routing::post,
};
use graph_api_support::{TOKEN, app_with, send_as};
use serde_json::{Value, json};

#[derive(Clone, Default)]
struct Stub {
    seen: Arc<Mutex<Vec<(Option<String>, Value)>>>,
    answer: Arc<Mutex<(u16, Value)>>,
    delay_ms: Arc<Mutex<u64>>,
}

async fn start_stub(stub: Stub) -> String {
    let router = Router::new()
        .route(
            "/v1/chat/completions",
            post(
                |State(stub): State<Stub>, headers: HeaderMap, Json(body): Json<Value>| async move {
                    let key = headers
                        .get("authorization")
                        .map(|value| value.to_str().unwrap().to_owned());
                    stub.seen.lock().unwrap().push((key, body));
                    let delay = *stub.delay_ms.lock().unwrap();
                    if delay > 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    }
                    let (status, answer) = stub.answer.lock().unwrap().clone();
                    (StatusCode::from_u16(status).unwrap(), Json(answer))
                },
            ),
        )
        .with_state(stub);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    url
}

fn answer(text: &str) -> Value {
    json!({
        "model": "stub-1",
        "choices": [{"message": {"role": "assistant", "content": text}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 12, "completion_tokens": 3}
    })
}

async fn app_with_stub(stub: &Stub, extra: &[(&str, &str)]) -> axum::Router {
    let endpoint = start_stub(stub.clone()).await;
    let mut env = vec![
        ("CORROBORE_MODEL_ENDPOINT", endpoint.as_str()),
        ("CORROBORE_MODEL_NAME", "stub-1"),
        ("CORROBORE_MODEL_API_KEY", "model-secret-key"),
        ("CORROBORE_MODEL_MAX_TOKENS", "256"),
    ];
    env.extend_from_slice(extra);
    app_with(None, &env)
}

async fn complete(app: &axum::Router, body: Value) -> (StatusCode, Value) {
    send_as(
        app,
        TOKEN,
        Method::POST,
        "/v1/model/complete",
        Some(body),
        &[],
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_completion_relays_messages_and_keeps_the_key_server_side() {
    let stub = Stub::default();
    *stub.answer.lock().unwrap() = (200, answer("Three indicators changed."));
    let app = app_with_stub(&stub, &[]).await;
    let (status, body) = complete(
        &app,
        json!({"messages": [
            {"role": "system", "content": "Summarize graph changes."},
            {"role": "user", "content": "alpha, beta, gamma"}
        ], "max_tokens": 10_000, "temperature": 0.2}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["text"], "Three indicators changed.");
    assert_eq!(body["result"]["model"], "stub-1");
    assert_eq!(body["result"]["prompt_tokens"], 12);
    assert!(!body.to_string().contains("model-secret-key"));
    let (key, request) = stub.seen.lock().unwrap()[0].clone();
    assert_eq!(key.as_deref(), Some("Bearer model-secret-key"));
    assert_eq!(request["model"], "stub-1");
    assert_eq!(request["max_tokens"], 256, "capped by the server");
    assert_eq!(request["messages"][1]["content"], "alpha, beta, gamma");

    let (status, body) = send_as(&app, TOKEN, Method::GET, "/v1/model", None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["result"],
        json!({"configured": true, "model": "stub-1", "max_tokens": 256})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn json_mode_requires_a_json_object() {
    let stub = Stub::default();
    *stub.answer.lock().unwrap() = (
        200,
        answer(r#"{"property": "score", "op": ">", "value": 3}"#),
    );
    let app = app_with_stub(&stub, &[]).await;
    let request =
        json!({"messages": [{"role": "user", "content": "score above three"}], "json": true});
    let (status, body) = complete(&app, request.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        stub.seen.lock().unwrap()[0].1["response_format"],
        json!({"type": "json_object"})
    );
    *stub.answer.lock().unwrap() = (200, answer("not json"));
    let (status, body) = complete(&app, request).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_failures_and_timeouts_are_mapped() {
    let stub = Stub::default();
    *stub.answer.lock().unwrap() = (429, json!({"error": {"message": "rate limited"}}));
    let app = app_with_stub(&stub, &[("CORROBORE_MODEL_TIMEOUT_MS", "300")]).await;
    let request = json!({"messages": [{"role": "user", "content": "hi"}]});
    let (status, body) = complete(&app, request.clone()).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["error"]["code"], "MODEL_PROVIDER_FAILED");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("rate limited")
    );
    *stub.answer.lock().unwrap() = (200, answer("late"));
    *stub.delay_ms.lock().unwrap() = 1_000;
    let (status, body) = complete(&app, request).await;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn requests_are_validated_before_any_call() {
    let stub = Stub::default();
    *stub.answer.lock().unwrap() = (200, answer("x"));
    let app = app_with_stub(&stub, &[]).await;
    for body in [
        json!({"messages": []}),
        json!({"messages": [{"role": "assistant", "content": "x"}]}),
        json!({"messages": [{"role": "user", "content": "x"}], "temperature": 7.0}),
        json!({"messages": [{"role": "tool", "content": "x"}]}),
    ] {
        let (status, _) = complete(&app, body.clone()).await;
        assert!(status.is_client_error(), "{body}");
    }
    assert!(stub.seen.lock().unwrap().is_empty());

    let app = app_with(None, &[]);
    let (status, body) = complete(
        &app,
        json!({"messages": [{"role": "user", "content": "x"}]}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "MODEL_NOT_CONFIGURED");
    let (_, body) = send_as(&app, TOKEN, Method::GET, "/v1/model", None, &[]).await;
    assert_eq!(body["result"]["configured"], false);
}

#[test]
fn model_configuration_is_validated() {
    use corrobore_http_server::ServerConfig;
    let config = |extra: &[(&str, &str)]| {
        let mut vars = std::collections::HashMap::from([(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            TOKEN.to_owned(),
        )]);
        for (name, value) in extra {
            vars.insert((*name).to_owned(), (*value).to_owned());
        }
        ServerConfig::from_map(&vars).map_err(|error| error.to_string())
    };
    assert!(config(&[]).unwrap().model.is_none());
    let error = config(&[(
        "CORROBORE_MODEL_ENDPOINT",
        "http://localhost:11434/v1/chat/completions",
    )])
    .unwrap_err();
    assert!(error.contains("CORROBORE_MODEL_NAME"), "{error}");
    let error = config(&[("CORROBORE_MODEL_ENDPOINT", "ftp://models.example")]).unwrap_err();
    assert!(error.contains("CORROBORE_MODEL_ENDPOINT"), "{error}");
    let error = config(&[("CORROBORE_MODEL_API_KEY", "orphan-key")]).unwrap_err();
    assert!(error.contains("CORROBORE_MODEL_API_KEY"), "{error}");
    let model = config(&[
        (
            "CORROBORE_MODEL_ENDPOINT",
            "https://models.example/v1/chat/completions",
        ),
        ("CORROBORE_MODEL_NAME", "small"),
        ("CORROBORE_MODEL_API_KEY", "model-secret-key"),
    ])
    .unwrap()
    .model
    .unwrap();
    assert_eq!(model.max_tokens, 1024);
    assert_eq!(model.timeout_ms, 60_000);
    assert!(
        !format!("{model:?}").contains("model-secret-key"),
        "the key is redacted"
    );
}
