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

//! Embedding providers for the vector and hybrid seed modes (issue #343).
//!
//! A provider turns text into vectors. Nodes carry vectors from the same
//! provider in their `embedding` property, written by clients that call
//! `POST /v1/embeddings`; seed search embeds the objective with it and asks
//! the canonical vector and hybrid retrieval for the nearest nodes.
//!
//! - [`HashingEmbeddingProvider`] needs no model: it hashes words and
//!   character trigrams into a fixed-size, unit-length vector. Similarity is
//!   lexical, not semantic, and fully deterministic.
//! - [`HttpEmbeddingProvider`] calls an OpenAI-compatible `/v1/embeddings`
//!   endpoint (a hosted API, or a local Ollama, TEI or vLLM server) for a real
//!   embedding model.
//!
//! Both block: callers run them on a blocking thread.

use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::config::{EmbeddingConfig, EmbeddingProviderKind};

/// Why text could not be embedded.
#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    /// The input cannot be embedded.
    #[error("{0}")]
    Invalid(String),
    /// The provider failed or answered something unusable.
    #[error("embedding provider failed: {0}")]
    Provider(String),
}

/// Turns text into vectors.
pub trait EmbeddingProvider: Send + Sync + std::fmt::Debug {
    /// Name reported with results: `hashing` or `http:<model>`.
    fn name(&self) -> String;

    /// One vector per text, in order.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::Invalid`] for empty input and
    /// [`EmbeddingError::Provider`] when the provider fails.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError>;
}

/// Build the provider the configuration selects, if any.
#[must_use]
pub fn provider_from_config(config: &EmbeddingConfig) -> Option<Box<dyn EmbeddingProvider>> {
    match config.provider {
        EmbeddingProviderKind::None => None,
        EmbeddingProviderKind::Hashing => {
            Some(Box::new(HashingEmbeddingProvider::new(config.dimensions)))
        }
        EmbeddingProviderKind::Http => Some(Box::new(HttpEmbeddingProvider {
            endpoint: config.endpoint.clone().unwrap_or_default(),
            model: config.model.clone().unwrap_or_default(),
            api_key: config.api_key.clone(),
            timeout: Duration::from_millis(config.timeout_ms),
        })),
    }
}

/// Feature hashing of lower-cased words and their character trigrams.
#[derive(Clone, Debug)]
pub struct HashingEmbeddingProvider {
    dimensions: usize,
}

const WORD_WEIGHT: f32 = 1.0;
const TRIGRAM_WEIGHT: f32 = 0.5;

impl HashingEmbeddingProvider {
    /// A provider producing `dimensions`-long vectors.
    #[must_use]
    pub fn new(dimensions: usize) -> Self {
        Self {
            dimensions: dimensions.max(1),
        }
    }

    fn embed_one(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let mut vector = vec![0.0_f32; self.dimensions];
        let lowered = text.to_lowercase();
        let words = lowered
            .split(|character: char| !character.is_alphanumeric())
            .filter(|word| word.chars().count() >= 2);
        let mut any = false;
        for word in words {
            any = true;
            self.add(&mut vector, "w", word, WORD_WEIGHT);
            let padded: Vec<char> = format!("^{word}$").chars().collect();
            for trigram in padded.windows(3) {
                let trigram: String = trigram.iter().collect();
                self.add(&mut vector, "t", &trigram, TRIGRAM_WEIGHT);
            }
        }
        let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        if !any || norm == 0.0 {
            return Err(EmbeddingError::Invalid(
                "text has no word of two or more letters or digits to embed".to_owned(),
            ));
        }
        for value in &mut vector {
            *value /= norm;
        }
        Ok(vector)
    }

    fn add(&self, vector: &mut [f32], kind: &str, feature: &str, weight: f32) {
        let hash = fnv1a(kind.as_bytes(), feature.as_bytes());
        let index = (hash % self.dimensions as u64) as usize;
        let sign = if hash >> 63 == 0 { 1.0 } else { -1.0 };
        vector[index] += sign * weight;
    }
}

/// 64-bit FNV-1a, stable across platforms and releases.
fn fnv1a(kind: &[u8], feature: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in kind.iter().chain(b":").chain(feature) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

impl EmbeddingProvider for HashingEmbeddingProvider {
    fn name(&self) -> String {
        "hashing".to_owned()
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        texts.iter().map(|text| self.embed_one(text)).collect()
    }
}

/// An OpenAI-compatible `/v1/embeddings` endpoint.
#[derive(Clone)]
pub struct HttpEmbeddingProvider {
    endpoint: String,
    model: String,
    api_key: Option<String>,
    timeout: Duration,
}

impl std::fmt::Debug for HttpEmbeddingProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HttpEmbeddingProvider")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("timeout", &self.timeout)
            .finish()
    }
}

#[derive(Deserialize)]
struct EmbeddingsResponse {
    data: Vec<EmbeddingItem>,
}

#[derive(Deserialize)]
struct EmbeddingItem {
    index: usize,
    embedding: Vec<f32>,
}

impl EmbeddingProvider for HttpEmbeddingProvider {
    fn name(&self) -> String {
        format!("http:{}", self.model)
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        // The blocking client owns a runtime, so it is built and dropped here,
        // on the caller's blocking thread, never on an async worker.
        let client = reqwest::blocking::Client::builder()
            .timeout(self.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| EmbeddingError::Provider(error.to_string()))?;
        let mut request = client
            .post(&self.endpoint)
            .json(&json!({ "model": self.model, "input": texts }));
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        let response = request
            .send()
            .map_err(|error| EmbeddingError::Provider(error.without_url().to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(EmbeddingError::Provider(format!(
                "endpoint answered {status}"
            )));
        }
        let mut body: EmbeddingsResponse = response
            .json()
            .map_err(|error| EmbeddingError::Provider(format!("unreadable answer: {error}")))?;
        body.data.sort_by_key(|item| item.index);
        if body.data.len() != texts.len()
            || body
                .data
                .iter()
                .enumerate()
                .any(|(index, item)| item.index != index)
        {
            return Err(EmbeddingError::Provider(format!(
                "expected {} embeddings, got {}",
                texts.len(),
                body.data.len()
            )));
        }
        let vectors: Vec<Vec<f32>> = body.data.into_iter().map(|item| item.embedding).collect();
        if vectors
            .iter()
            .any(|vector| vector.is_empty() || vector.iter().any(|value| !value.is_finite()))
        {
            return Err(EmbeddingError::Provider(
                "an embedding is empty or not finite".to_owned(),
            ));
        }
        Ok(vectors)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{EmbeddingProvider, HashingEmbeddingProvider};

    fn cosine(left: &[f32], right: &[f32]) -> f32 {
        left.iter().zip(right).map(|(a, b)| a * b).sum()
    }

    #[test]
    fn shared_words_make_texts_closer() {
        let provider = HashingEmbeddingProvider::new(256);
        let vectors = provider
            .embed(&[
                "phishing campaign".to_owned(),
                "acme phishing campaign targeting banks".to_owned(),
                "ransomware loader".to_owned(),
            ])
            .unwrap();
        assert!(cosine(&vectors[0], &vectors[1]) > cosine(&vectors[0], &vectors[2]));
    }

    #[test]
    fn text_without_words_is_refused() {
        let provider = HashingEmbeddingProvider::new(16);
        assert!(provider.embed(&["  - . ".to_owned()]).is_err());
    }
}
