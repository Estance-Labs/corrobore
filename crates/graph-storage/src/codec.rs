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
use crate::{
    GraphStorageError, GraphStorageResult, PersistedRecordEnvelope, PersistedRecordKind,
    RecordChecksum, RecordFormat, StorageVersion, validate_persisted_record_envelope,
};

const CHECKSUM_ALGORITHM_SHA256: &str = "sha256";

/// Encoded persisted record unit produced by a `RecordCodec`.
///
///
/// - Represent the opaque byte output of a deterministic record codec without
///   exposing the concrete serialization format as the long-term public API.
/// - Carry enough compatibility metadata for later append-only storage readers to
///   reject unsupported versions, unexpected record kinds, or corrupted bytes
///   before trusting the decoded envelope.
/// - Keep checksum ownership with the encoded unit so fixtures can remain stable
///   and reproducible across test runs.
///
///
/// `RecordCodec::encode_envelope` returns this value after producing canonical
/// bytes and calculating the checksum for those bytes. `RecordCodec::decode_envelope`
/// validates the checksum before returning a `PersistedRecordEnvelope`.
///
/// # Errors
///
///
/// Invalid versions, unsupported formats, checksum mismatches, decode failures,
/// and unexpected record kinds must be returned as explicit `GraphStorageError`
/// variants rather than silently producing empty or partially trusted records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedRecord {
    /// Storage compatibility version associated with the encoded bytes.
    pub storage_version: StorageVersion,

    /// Durable record format used by the encoded bytes.
    pub record_format: RecordFormat,

    /// Logical record kind expected to be recovered from the encoded bytes.
    pub kind: PersistedRecordKind,

    /// Opaque encoded bytes. Callers must not depend on the concrete layout.
    pub bytes: Vec<u8>,

    /// Checksum calculated over the canonical encoded bytes.
    pub checksum: RecordChecksum,
}

/// Codec boundary for deterministic persisted record encoding and decoding.
///
///
/// - Define the public contract for converting persisted record envelopes to and
///   from durable bytes.
/// - Keep storage-root, append-only log, and pager code independent from the
///   first concrete serialization implementation.
/// - Make checksum calculation and validation part of the codec contract so
///   corrupted data is rejected before it can be used by graph loading code.
///
///
/// Implementations must produce deterministic bytes for the same envelope,
/// calculate deterministic checksums, validate checksums during decode, and
/// preserve the envelope metadata needed for node, relationship, and adjacency
/// records.
///
/// # Errors
///
///
/// Implementations must return explicit errors for checksum mismatch, decode
/// failure, unexpected record kind, unsupported storage version, unsupported
/// record format, and invalid envelopes.
pub trait RecordCodec: Send + Sync {
    /// Encode a trusted persisted record envelope into deterministic bytes.
    ///
    ///
    /// - Convert an already constructed `PersistedRecordEnvelope` into an opaque
    ///   durable representation.
    /// - Calculate the checksum over the canonical encoded bytes.
    /// - Avoid exposing the concrete serialization details to callers.
    ///
    ///
    ///   The same envelope, storage version, and record format must always produce
    ///   the same `EncodedRecord` bytes and checksum.
    ///
    /// # Errors
    ///
    ///
    /// Invalid envelope metadata, unsupported versions, unsupported formats, or
    /// codec failures must be surfaced as `GraphStorageError` variants.
    fn encode_envelope(
        &self,
        envelope: &PersistedRecordEnvelope,
    ) -> GraphStorageResult<EncodedRecord>;

    /// Decode deterministic bytes back into a persisted record envelope.
    ///
    ///
    /// - Rehydrate an envelope from bytes produced by a compatible codec.
    /// - Validate the encoded checksum before trusting decoded metadata.
    /// - Optionally enforce the record kind expected by the caller.
    ///
    ///
    ///   Decoding must either return a fully validated `PersistedRecordEnvelope` or
    ///   fail with a typed error. It must not return empty fallback records.
    ///
    /// # Errors
    ///
    ///
    /// Checksum mismatch, unsupported storage version, unsupported record format,
    /// malformed bytes, invalid envelopes, and unexpected record kinds must be
    /// explicit errors.
    fn decode_envelope(
        &self,
        encoded_record: &EncodedRecord,
        expected_kind: Option<PersistedRecordKind>,
    ) -> GraphStorageResult<PersistedRecordEnvelope>;

    /// Calculate the checksum for canonical encoded bytes.
    ///
    ///
    /// - Provide one storage-level checksum contract for encoded record bytes.
    /// - Keep the checksum algorithm choice behind the codec boundary for now.
    /// - Make future fixtures deterministic and easy to compare.
    ///
    ///
    ///   The same byte slice must always produce the same `RecordChecksum`.
    ///
    /// # Errors
    ///
    ///
    /// Unsupported checksum algorithms or lower-level checksum failures must be
    /// mapped to `GraphStorageError`.
    fn calculate_checksum(&self, bytes: &[u8]) -> GraphStorageResult<RecordChecksum>;

    /// Validate encoded bytes against an expected checksum.
    ///
    ///
    /// - Detect corrupted persisted records before decode output is trusted.
    /// - Make checksum mismatch a first-class storage error.
    /// - Keep record-load code from duplicating checksum behavior.
    ///
    ///
    ///   The function returns `Ok(())` only when the calculated checksum matches the
    ///   expected checksum exactly.
    ///
    /// # Errors
    ///
    ///
    /// A mismatch must return `GraphStorageError::ChecksumMismatch`; unsupported
    /// algorithms or calculation failures must remain explicit errors.
    fn validate_checksum(
        &self,
        bytes: &[u8],
        expected_checksum: &RecordChecksum,
    ) -> GraphStorageResult<()>;
}

/// Deterministic JSON Lines codec for persisted record envelopes.
///
///
/// - Provide the first concrete record codec without making JSON an irreversible
///   long-term public storage API.
/// - Use a simple line-framed representation suitable for stable fixtures and
///   early append-only storage records.
/// - Keep checksum validation and envelope validation inside the codec boundary.
///
///
///   The codec serializes one `PersistedRecordEnvelope` as one canonical JSON value
///   followed by a newline, calculates a SHA-256 checksum over those exact bytes,
///   validates the checksum before decode, and returns only fully validated
///   envelopes.
///
/// # Errors
///
///
/// Unsupported versions, unsupported formats, checksum mismatches, malformed
/// bytes, invalid envelopes, and unexpected record kinds are returned as typed
/// `GraphStorageError` variants.
#[derive(Clone, Debug, Default)]
pub struct JsonLinesRecordCodec;

impl RecordCodec for JsonLinesRecordCodec {
    fn encode_envelope(
        &self,
        envelope: &PersistedRecordEnvelope,
    ) -> GraphStorageResult<EncodedRecord> {
        validate_persisted_record_envelope(envelope)?;
        validate_storage_version(&envelope.storage_version)?;
        validate_record_format(&envelope.record_format)?;

        let mut bytes =
            serde_json::to_vec(envelope).map_err(|error| GraphStorageError::OperationFailed {
                operation: "JsonLinesRecordCodec::encode_envelope",
                message: error.to_string(),
            })?;
        bytes.push(b'\n');

        let checksum = self.calculate_checksum(&bytes)?;

        Ok(EncodedRecord {
            storage_version: envelope.storage_version.clone(),
            record_format: envelope.record_format.clone(),
            kind: envelope.kind,
            bytes,
            checksum,
        })
    }

    fn decode_envelope(
        &self,
        encoded_record: &EncodedRecord,
        expected_kind: Option<PersistedRecordKind>,
    ) -> GraphStorageResult<PersistedRecordEnvelope> {
        validate_storage_version(&encoded_record.storage_version)?;
        validate_record_format(&encoded_record.record_format)?;
        self.validate_checksum(&encoded_record.bytes, &encoded_record.checksum)?;

        let envelope: PersistedRecordEnvelope = serde_json::from_slice(&encoded_record.bytes)
            .map_err(|error| GraphStorageError::DecodeFailed {
                format: record_format_name(&encoded_record.record_format),
                reason: error.to_string(),
            })?;

        validate_persisted_record_envelope(&envelope)?;
        validate_storage_version(&envelope.storage_version)?;
        validate_record_format(&envelope.record_format)?;

        if envelope.storage_version != encoded_record.storage_version {
            return Err(GraphStorageError::DecodeFailed {
                format: record_format_name(&encoded_record.record_format),
                reason: format!(
                    "encoded storage version {:?} does not match envelope storage version {:?}",
                    encoded_record.storage_version, envelope.storage_version
                ),
            });
        }

        if envelope.record_format != encoded_record.record_format {
            return Err(GraphStorageError::DecodeFailed {
                format: record_format_name(&encoded_record.record_format),
                reason: format!(
                    "encoded record format {:?} does not match envelope record format {:?}",
                    encoded_record.record_format, envelope.record_format
                ),
            });
        }

        if envelope.kind != encoded_record.kind {
            return Err(GraphStorageError::UnexpectedRecordKind {
                expected: encoded_record.kind,
                actual: envelope.kind,
            });
        }

        if let Some(expected_kind) = expected_kind
            && envelope.kind != expected_kind
        {
            return Err(GraphStorageError::UnexpectedRecordKind {
                expected: expected_kind,
                actual: envelope.kind,
            });
        }

        Ok(envelope)
    }

    fn calculate_checksum(&self, bytes: &[u8]) -> GraphStorageResult<RecordChecksum> {
        Ok(RecordChecksum {
            algorithm: CHECKSUM_ALGORITHM_SHA256.to_owned(),
            value: encode_lower_hex(&sha256_digest(bytes)),
        })
    }

    fn validate_checksum(
        &self,
        bytes: &[u8],
        expected_checksum: &RecordChecksum,
    ) -> GraphStorageResult<()> {
        let actual_checksum = self.calculate_checksum(bytes)?;
        if &actual_checksum == expected_checksum {
            Ok(())
        } else {
            Err(GraphStorageError::ChecksumMismatch {
                expected: expected_checksum.clone(),
                actual: actual_checksum,
            })
        }
    }
}

/// Encode a persisted record envelope with the supplied codec.
///
///
/// - Give storage writers a stable function boundary for encoding records.
/// - Keep the concrete codec implementation injectable for tests and future
///   format negotiation.
/// - Preserve deterministic behavior as a public expectation without exposing
///   serialization details.
///
///
/// The codec validates the envelope, produces canonical bytes, calculates the
/// checksum, and returns an `EncodedRecord` suitable for append-only persistence.
///
/// # Errors
///
///
/// Invalid envelopes, unsupported versions, unsupported formats, checksum
/// failures, and codec failures must be returned as `GraphStorageError`.
pub fn encode_persisted_record_envelope(
    codec: &dyn RecordCodec,
    envelope: &PersistedRecordEnvelope,
) -> GraphStorageResult<EncodedRecord> {
    codec.encode_envelope(envelope)
}

/// Decode a persisted record envelope with the supplied codec.
///
///
/// - Give storage readers and future record loaders a stable function boundary
///   for decoding persisted bytes.
/// - Validate corruption before decoded metadata is trusted.
/// - Allow callers to require a node, relationship, or adjacency envelope when
///   reading from a typed storage segment.
///
///
/// The codec validates the checksum, decodes canonical bytes, validates the
/// resulting envelope, and checks `expected_kind` when provided.
///
/// # Errors
///
///
/// Checksum mismatch, decode failure, invalid envelope metadata, unexpected
/// record kind, unsupported storage version, and unsupported record format must
/// be returned explicitly.
pub fn decode_persisted_record_envelope(
    codec: &dyn RecordCodec,
    encoded_record: &EncodedRecord,
    expected_kind: Option<PersistedRecordKind>,
) -> GraphStorageResult<PersistedRecordEnvelope> {
    codec.decode_envelope(encoded_record, expected_kind)
}

/// Calculate a checksum for canonical encoded record bytes.
///
///
/// - Centralize checksum calculation behind the codec boundary.
/// - Provide stable fixture support for encoded records.
/// - Avoid duplicating checksum algorithm decisions in storage writers.
///
///
///   The same bytes must always produce the same checksum for the same codec.
///
/// # Errors
///
///
/// Unsupported algorithms or lower-level checksum failures must be mapped to
/// typed `GraphStorageError` values.
pub fn calculate_encoded_record_checksum(
    codec: &dyn RecordCodec,
    bytes: &[u8],
) -> GraphStorageResult<RecordChecksum> {
    codec.calculate_checksum(bytes)
}

/// Validate encoded record bytes against their expected checksum.
///
///
/// - Provide the future record-load path with a single checksum validation entry
///   point.
/// - Ensure corrupted records are rejected before decode output is trusted.
/// - Make mismatch behavior deterministic and directly testable.
///
///
///   The supplied codec recalculates the checksum for `bytes` and compares it with
///   `expected_checksum`.
///
/// # Errors
///
///
/// Mismatches must return `GraphStorageError::ChecksumMismatch`; checksum
/// calculation failures must remain explicit storage errors.
pub fn validate_encoded_record_checksum(
    codec: &dyn RecordCodec,
    bytes: &[u8],
    expected_checksum: &RecordChecksum,
) -> GraphStorageResult<()> {
    codec.validate_checksum(bytes, expected_checksum)
}

fn validate_storage_version(storage_version: &StorageVersion) -> GraphStorageResult<()> {
    match storage_version {
        StorageVersion::V1 => Ok(()),
        StorageVersion::Unsupported(version) => Err(GraphStorageError::UnsupportedStorageVersion {
            version: version.clone(),
        }),
    }
}

fn validate_record_format(record_format: &RecordFormat) -> GraphStorageResult<()> {
    match record_format {
        RecordFormat::JsonLinesV1 => Ok(()),
        RecordFormat::Unsupported(format) => Err(GraphStorageError::UnsupportedRecordFormat {
            format: format.clone(),
        }),
    }
}

fn record_format_name(record_format: &RecordFormat) -> String {
    match record_format {
        RecordFormat::JsonLinesV1 => "JsonLinesV1".to_owned(),
        RecordFormat::Unsupported(format) => format.clone(),
    }
}

fn encode_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn sha256_digest(input: &[u8]) -> [u8; 32] {
    const INITIAL_HASH: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    const ROUND_CONSTANTS: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let mut message = input.to_vec();
    let bit_len = (message.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    let mut hash = INITIAL_HASH;
    for chunk in message.chunks_exact(64) {
        let mut schedule = [0u32; 64];
        for (index, word) in schedule.iter_mut().take(16).enumerate() {
            let offset = index * 4;
            *word = u32::from_be_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }

        for index in 16..64 {
            let s0 = schedule[index - 15].rotate_right(7)
                ^ schedule[index - 15].rotate_right(18)
                ^ (schedule[index - 15] >> 3);
            let s1 = schedule[index - 2].rotate_right(17)
                ^ schedule[index - 2].rotate_right(19)
                ^ (schedule[index - 2] >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(s0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(s1);
        }

        let mut a = hash[0];
        let mut b = hash[1];
        let mut c = hash[2];
        let mut d = hash[3];
        let mut e = hash[4];
        let mut f = hash[5];
        let mut g = hash[6];
        let mut h = hash[7];

        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(ROUND_CONSTANTS[index])
                .wrapping_add(schedule[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        hash[0] = hash[0].wrapping_add(a);
        hash[1] = hash[1].wrapping_add(b);
        hash[2] = hash[2].wrapping_add(c);
        hash[3] = hash[3].wrapping_add(d);
        hash[4] = hash[4].wrapping_add(e);
        hash[5] = hash[5].wrapping_add(f);
        hash[6] = hash[6].wrapping_add(g);
        hash[7] = hash[7].wrapping_add(h);
    }

    let mut output = [0u8; 32];
    for (index, word) in hash.iter().enumerate() {
        output[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StorageRef, StorageSegment, create_node_record_envelope};
    use graph_core::{Graph, NodeInput, PropertyValue};

    fn node_envelope() -> PersistedRecordEnvelope {
        let mut graph = Graph::new();
        let node_id = graph
            .create_node(NodeInput::new(["Campaign", "FIMI"]))
            .expect("node creation should succeed");
        let node = graph
            .get_node(&node_id)
            .expect("node lookup should succeed")
            .expect("node should exist");

        create_node_record_envelope(
            &node,
            StorageRef {
                // Segment.
                segment: StorageSegment::NodeRecords,
                // Offset.
                offset: 12,
                // Length.
                length: 34,
                // Checksum.
                checksum: None,
            },
            StorageVersion::V1,
            RecordFormat::JsonLinesV1,
            None,
        )
        .expect("node envelope should be valid")
    }

    #[test]
    fn decode_rejects_when_encoded_kind_does_not_match_decoded_envelope_kind() {
        let codec = JsonLinesRecordCodec;
        let envelope = node_envelope();
        let mut encoded = codec
            .encode_envelope(&envelope)
            .expect("encoding should succeed");
        encoded.kind = PersistedRecordKind::Relationship;

        let error = codec
            .decode_envelope(&encoded, None)
            .expect_err("encoded kind mismatch should be rejected");

        assert!(matches!(
            error,
            GraphStorageError::UnexpectedRecordKind {
                expected: PersistedRecordKind::Relationship,
                actual: PersistedRecordKind::Node,
            }
        ));
    }

    #[test]
    fn version_and_format_helpers_validate_supported_and_unsupported_values() {
        validate_storage_version(&StorageVersion::V1)
            .expect("supported storage version should be accepted");
        validate_record_format(&RecordFormat::JsonLinesV1)
            .expect("supported record format should be accepted");

        let version_error =
            validate_storage_version(&StorageVersion::Unsupported("V999".to_owned()))
                .expect_err("unsupported version should be rejected");
        let format_error =
            validate_record_format(&RecordFormat::Unsupported("BinaryV2".to_owned()))
                .expect_err("unsupported format should be rejected");

        assert!(matches!(
        version_error,
        GraphStorageError::UnsupportedStorageVersion { version } if version == "V999"
        ));
        assert!(matches!(
        format_error,
        GraphStorageError::UnsupportedRecordFormat { format } if format == "BinaryV2"
        ));
        assert_eq!(
            record_format_name(&RecordFormat::Unsupported("Custom".to_owned())),
            "Custom"
        );
    }

    #[test]
    fn hex_and_sha_helpers_are_deterministic() {
        assert_eq!(encode_lower_hex(&[0x00, 0x0f, 0x10, 0xff]), "000f10ff");

        let digest_a = sha256_digest(b"codec-determinism");
        let digest_b = sha256_digest(b"codec-determinism");
        let digest_c = sha256_digest(b"codec-determinism-changed");

        assert_eq!(digest_a, digest_b);
        assert_ne!(digest_a, digest_c);
        assert_eq!(digest_a.len(), 32);
    }

    #[test]
    fn decode_accepts_matching_expected_kind() {
        let codec = JsonLinesRecordCodec;
        let envelope = node_envelope();
        let encoded = codec
            .encode_envelope(&envelope)
            .expect("encoding should succeed");

        let decoded = codec
            .decode_envelope(&encoded, Some(PersistedRecordKind::Node))
            .expect("matching expected kind should decode");

        assert_eq!(decoded.kind, PersistedRecordKind::Node);
    }

    #[test]
    fn encode_accepts_non_finite_float_property_values() {
        let codec = JsonLinesRecordCodec;
        let mut graph = Graph::new();
        let node_id = graph
            .create_node(
                NodeInput::new(["Campaign", "FIMI"])
                    .with_property("score", PropertyValue::Float(f64::NAN)),
            )
            .expect("node creation with NaN property should succeed");
        let node = graph
            .get_node(&node_id)
            .expect("node lookup should succeed")
            .expect("node should exist");

        let envelope = create_node_record_envelope(
            &node,
            StorageRef {
                // Segment.
                segment: StorageSegment::NodeRecords,
                // Offset.
                offset: 12,
                // Length.
                length: 34,
                // Checksum.
                checksum: None,
            },
            StorageVersion::V1,
            RecordFormat::JsonLinesV1,
            None,
        )
        .expect("node envelope should be valid");

        let encoded = codec
            .encode_envelope(&envelope)
            .expect("encoding with non-finite float should currently succeed");
        assert!(!encoded.bytes.is_empty());
        assert_eq!(encoded.kind, PersistedRecordKind::Node);
    }
}
