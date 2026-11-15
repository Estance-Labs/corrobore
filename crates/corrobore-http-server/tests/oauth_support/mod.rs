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

//! A local OpenID Connect issuer for tests: discovery, a JWKS that can rotate,
//! and ES256 access tokens signed with its keys.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use axum::{Json, Router, extract::State, routing::get};
use base64::Engine as _;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde_json::{Value, json};

#[derive(Clone, Default)]
struct IssuerState {
    issuer: Arc<Mutex<String>>,
    keys: Arc<Mutex<Vec<Value>>>,
    jwks_fetches: Arc<AtomicUsize>,
    discovery_path: Arc<Mutex<&'static str>>,
}

/// One signing key of the issuer.
pub struct SigningKey {
    pub kid: String,
    der: Vec<u8>,
}

pub struct TestIssuer {
    state: IssuerState,
    url: String,
    signing: Mutex<Vec<SigningKey>>,
}

impl TestIssuer {
    /// Start an issuer serving OIDC discovery and a JWKS with one key `k1`.
    pub async fn start() -> Self {
        Self::start_with_discovery("/.well-known/openid-configuration").await
    }

    /// Start an issuer whose discovery document lives at `path` only, such as
    /// the RFC 8414 `/.well-known/oauth-authorization-server`.
    pub async fn start_with_discovery(path: &'static str) -> Self {
        let state = IssuerState::default();
        *state.discovery_path.lock().unwrap() = path;
        let router = Router::new()
            .route(
                "/.well-known/openid-configuration",
                get(|State(state): State<IssuerState>| async move {
                    discovery(&state, "/.well-known/openid-configuration")
                }),
            )
            .route(
                "/.well-known/oauth-authorization-server",
                get(|State(state): State<IssuerState>| async move {
                    discovery(&state, "/.well-known/oauth-authorization-server")
                }),
            )
            .route(
                "/jwks",
                get(|State(state): State<IssuerState>| async move {
                    state.jwks_fetches.fetch_add(1, Ordering::SeqCst);
                    Json(json!({"keys": state.keys.lock().unwrap().clone()}))
                }),
            )
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        *state.issuer.lock().unwrap() = url.clone();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let issuer = Self {
            state,
            url,
            signing: Mutex::new(Vec::new()),
        };
        issuer.add_key("k1");
        issuer
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn jwks_url(&self) -> String {
        format!("{}/jwks", self.url)
    }

    pub fn jwks_fetches(&self) -> usize {
        self.state.jwks_fetches.load(Ordering::SeqCst)
    }

    /// Publish a new P-256 key under `kid`.
    pub fn add_key(&self, kid: &str) {
        let pair = rcgen::KeyPair::generate().unwrap();
        let raw = pair.public_key_raw();
        assert_eq!(raw.len(), 65, "uncompressed P-256 point");
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        self.state.keys.lock().unwrap().push(json!({
            "kty": "EC",
            "crv": "P-256",
            "use": "sig",
            "alg": "ES256",
            "kid": kid,
            "x": b64.encode(&raw[1..33]),
            "y": b64.encode(&raw[33..65]),
        }));
        self.signing.lock().unwrap().push(SigningKey {
            kid: kid.to_owned(),
            der: pair.serialize_der(),
        });
    }

    /// Claims of a valid access token for `sub`, issued now for five minutes.
    pub fn claims(&self, sub: &str, audience: &str) -> Value {
        let now = now();
        json!({
            "iss": self.url,
            "sub": sub,
            "aud": audience,
            "iat": now,
            "nbf": now,
            "exp": now + 300,
            "client_id": "corrobore-viewer",
            "scope": "graph:read graph:write",
        })
    }

    /// Sign `claims` with the key `kid`.
    pub fn sign(&self, kid: &str, claims: &Value) -> String {
        let keys = self.signing.lock().unwrap();
        let key = keys.iter().find(|key| key.kid == kid).expect("known kid");
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(kid.to_owned());
        jsonwebtoken::encode(&header, claims, &EncodingKey::from_ec_der(&key.der)).unwrap()
    }

    /// A valid token for `sub` signed with `k1`.
    pub fn token(&self, sub: &str, audience: &str) -> String {
        self.sign("k1", &self.claims(sub, audience))
    }
}

fn discovery(state: &IssuerState, path: &str) -> axum::response::Response {
    use axum::response::IntoResponse;
    if *state.discovery_path.lock().unwrap() != path {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    }
    let issuer = state.issuer.lock().unwrap().clone();
    Json(json!({
        "issuer": issuer,
        "jwks_uri": format!("{issuer}/jwks"),
        "authorization_endpoint": format!("{issuer}/authorize"),
        "token_endpoint": format!("{issuer}/token"),
        "code_challenge_methods_supported": ["S256"],
        "response_types_supported": ["code"],
    }))
    .into_response()
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// An unsigned (`alg: none`) token carrying `claims`.
pub fn unsigned_token(claims: &Value) -> String {
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    format!(
        "{}.{}.",
        b64.encode(br#"{"alg":"none","typ":"JWT"}"#),
        b64.encode(claims.to_string())
    )
}

/// An HS256 token signed with `secret`, as an attacker who knows a public
/// key might forge one.
pub fn hmac_token(claims: &Value, secret: &[u8]) -> String {
    jsonwebtoken::encode(
        &Header::new(Algorithm::HS256),
        claims,
        &EncodingKey::from_secret(secret).unwrap(),
    )
    .unwrap()
}
