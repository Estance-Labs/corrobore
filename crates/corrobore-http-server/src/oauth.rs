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

//! OAuth 2.0 resource server (issue #382): verifies JWT access tokens issued
//! by one OpenID Connect or RFC 8414 authorization server.
//!
//! Keys come from the configured JWKS URL or from the issuer's discovery
//! document, are cached, and are fetched again when a token names a key id
//! the cache lacks (key rotation), at most once per [`REFRESH_INTERVAL`].
//! Only asymmetric algorithms are accepted: an `alg: none` or HMAC token can
//! never verify, whatever key material an attacker knows.

use std::time::{Duration, Instant};

use jsonwebtoken::{
    Algorithm, DecodingKey, Validation,
    jwk::{Jwk, JwkSet},
};
use serde::Deserialize;
use tokio::sync::RwLock;
use tracing::{debug, warn};

use crate::config::OAuthConfig;

/// Clock skew tolerated on `exp` and `nbf`.
const LEEWAY_SECONDS: u64 = 30;
/// Timeout of one discovery or key-set request.
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

const ACCEPTED_ALGORITHMS: &[Algorithm] = &[
    Algorithm::RS256,
    Algorithm::RS384,
    Algorithm::RS512,
    Algorithm::PS256,
    Algorithm::PS384,
    Algorithm::PS512,
    Algorithm::ES256,
    Algorithm::ES384,
    Algorithm::EdDSA,
];

/// The identity an access token proves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedToken {
    /// `principal:<sub>`.
    pub actor: String,
    /// `client_id`, else `azp`: the OAuth client the token was issued to.
    pub client_id: Option<String>,
    pub scopes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AccessClaims {
    sub: String,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    azp: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    scp: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct DiscoveryDocument {
    issuer: String,
    jwks_uri: String,
}

#[derive(Default)]
struct KeyCache {
    jwks_url: Option<String>,
    keys: Option<JwkSet>,
    fetched_at: Option<Instant>,
}

/// Verifies access tokens for one configured issuer and audience.
pub struct OAuthVerifier {
    config: OAuthConfig,
    client: reqwest::Client,
    cache: RwLock<KeyCache>,
}

impl std::fmt::Debug for OAuthVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthVerifier")
            .field("issuer", &self.config.issuer)
            .field("audience", &self.config.audience)
            .finish()
    }
}

impl OAuthVerifier {
    pub fn new(config: OAuthConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();
        let cache = KeyCache {
            jwks_url: config.jwks_url.clone(),
            ..KeyCache::default()
        };
        Self {
            config,
            client,
            cache: RwLock::new(cache),
        }
    }

    pub fn config(&self) -> &OAuthConfig {
        &self.config
    }

    /// Whether `token` has the shape of a JWS compact serialization.
    pub fn looks_like_jwt(token: &str) -> bool {
        token.split('.').count() == 3
    }

    /// Verify `token`; the error is a short reason for logs, never the token.
    pub async fn verify(&self, token: &str) -> Result<VerifiedToken, String> {
        let header =
            jsonwebtoken::decode_header(token).map_err(|_| "malformed token header".to_owned())?;
        if !ACCEPTED_ALGORITHMS.contains(&header.alg) {
            return Err(format!("algorithm {:?} is not accepted", header.alg));
        }
        let jwk = self
            .find_key(header.kid.as_deref(), header.alg)
            .await?
            .ok_or_else(|| "no published key matches the token".to_owned())?;
        let key = DecodingKey::from_jwk(&jwk).map_err(|_| "unusable published key".to_owned())?;
        let mut validation = Validation::new(header.alg);
        validation.leeway = LEEWAY_SECONDS;
        validation.validate_nbf = true;
        validation.set_issuer(&[self.config.issuer.as_str()]);
        validation.set_audience(&[self.config.audience.as_str()]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        let data = jsonwebtoken::decode::<AccessClaims>(token, &key, &validation)
            .map_err(|error| format!("token rejected: {:?}", error.kind()))?;
        let claims = data.claims;
        if claims.sub.trim().is_empty() {
            return Err("token has an empty subject".to_owned());
        }
        let client_id = claims.client_id.or(claims.azp);
        if !self.config.client_ids.is_empty()
            && !client_id
                .as_ref()
                .is_some_and(|client| self.config.client_ids.contains(client))
        {
            return Err("token was issued to a client that is not allowed".to_owned());
        }
        let scopes = match (claims.scope, claims.scp) {
            (Some(scope), _) => scope.split_whitespace().map(str::to_owned).collect(),
            (None, Some(scp)) => scp,
            (None, None) => Vec::new(),
        };
        Ok(VerifiedToken {
            actor: format!("principal:{}", claims.sub),
            client_id,
            scopes,
        })
    }

    /// The key for `kid` (or the only key usable with `alg` when the token
    /// names none). A miss fetches the key set again, at most once per
    /// refresh interval, so a rotated key is picked up on its first use.
    async fn find_key(&self, kid: Option<&str>, alg: Algorithm) -> Result<Option<Jwk>, String> {
        {
            let cache = self.cache.read().await;
            if let Some(keys) = &cache.keys {
                if let Some(key) = select_key(keys, kid, alg) {
                    return Ok(Some(key));
                }
                if !self.may_refresh(&cache) {
                    return Ok(None);
                }
            }
        }
        let mut cache = self.cache.write().await;
        // Another request may have refreshed while this one waited.
        if let Some(keys) = &cache.keys {
            if let Some(key) = select_key(keys, kid, alg) {
                return Ok(Some(key));
            }
            if !self.may_refresh(&cache) {
                return Ok(None);
            }
        }
        let jwks_url = match cache.jwks_url.clone() {
            Some(url) => url,
            None => {
                let url = self.discover_jwks_url().await?;
                cache.jwks_url = Some(url.clone());
                url
            }
        };
        let keys = self.fetch_keys(&jwks_url).await?;
        debug!(keys = keys.keys.len(), "oauth key set fetched");
        let selected = select_key(&keys, kid, alg);
        cache.keys = Some(keys);
        cache.fetched_at = Some(Instant::now());
        Ok(selected)
    }

    fn may_refresh(&self, cache: &KeyCache) -> bool {
        cache
            .fetched_at
            .is_none_or(|at| at.elapsed() >= self.config.jwks_min_refresh)
    }

    async fn fetch_keys(&self, url: &str) -> Result<JwkSet, String> {
        let response = self.client.get(url).send().await.map_err(|error| {
            warn!(error = %error, "oauth key set unreachable");
            "the issuer's key set is unreachable".to_owned()
        })?;
        if !response.status().is_success() {
            return Err(format!(
                "the issuer's key set answered {}",
                response.status()
            ));
        }
        response
            .json::<JwkSet>()
            .await
            .map_err(|_| "the issuer's key set is not a JWK set".to_owned())
    }

    /// OIDC discovery first, then RFC 8414 authorization-server metadata.
    /// The document must name the configured issuer exactly.
    async fn discover_jwks_url(&self) -> Result<String, String> {
        for url in discovery_urls(&self.config.issuer) {
            let Ok(response) = self.client.get(&url).send().await else {
                continue;
            };
            if !response.status().is_success() {
                continue;
            }
            let Ok(document) = response.json::<DiscoveryDocument>().await else {
                continue;
            };
            if document.issuer.trim_end_matches('/') != self.config.issuer {
                warn!(url = %url, "oauth discovery names another issuer");
                continue;
            }
            return Ok(document.jwks_uri);
        }
        Err("the issuer publishes no usable discovery document".to_owned())
    }
}

/// `{issuer}/.well-known/openid-configuration`, then the RFC 8414 location
/// that inserts the suffix between host and path.
fn discovery_urls(issuer: &str) -> Vec<String> {
    let mut urls = vec![format!("{issuer}/.well-known/openid-configuration")];
    if let Ok(parsed) = url::Url::parse(issuer) {
        let path = parsed.path().trim_end_matches('/');
        urls.push(format!(
            "{}/.well-known/oauth-authorization-server{}",
            parsed.origin().ascii_serialization(),
            path
        ));
    }
    urls
}

fn select_key(keys: &JwkSet, kid: Option<&str>, alg: Algorithm) -> Option<Jwk> {
    let compatible = |jwk: &&Jwk| key_accepts(jwk, alg);
    match kid {
        Some(kid) => keys
            .keys
            .iter()
            .filter(|jwk| jwk.common.key_id.as_deref() == Some(kid))
            .find(compatible)
            .cloned(),
        None => {
            let mut candidates = keys.keys.iter().filter(compatible);
            let first = candidates.next()?;
            // Without a key id, only an unambiguous key is used.
            candidates.next().is_none().then(|| first.clone())
        }
    }
}

/// A key declares the algorithm it serves (when it says) and must be of a
/// type that algorithm uses, so an RSA key never verifies an EC signature.
fn key_accepts(jwk: &Jwk, alg: Algorithm) -> bool {
    use jsonwebtoken::jwk::AlgorithmParameters;
    if let Some(declared) = &jwk.common.key_algorithm {
        let declared = serde_json::to_value(declared).ok();
        let wanted = serde_json::to_value(alg).ok();
        if declared != wanted {
            return false;
        }
    }
    matches!(
        (&jwk.algorithm, alg),
        (
            AlgorithmParameters::RSA(_),
            Algorithm::RS256
                | Algorithm::RS384
                | Algorithm::RS512
                | Algorithm::PS256
                | Algorithm::PS384
                | Algorithm::PS512
        ) | (
            AlgorithmParameters::EllipticCurve(_),
            Algorithm::ES256 | Algorithm::ES384
        ) | (AlgorithmParameters::OctetKeyPair(_), Algorithm::EdDSA)
    )
}

#[cfg(test)]
mod tests {
    use super::discovery_urls;

    #[test]
    fn discovery_covers_oidc_and_rfc8414_locations() {
        assert_eq!(
            discovery_urls("https://id.example/realms/corrobore"),
            [
                "https://id.example/realms/corrobore/.well-known/openid-configuration",
                "https://id.example/.well-known/oauth-authorization-server/realms/corrobore",
            ]
        );
    }
}
