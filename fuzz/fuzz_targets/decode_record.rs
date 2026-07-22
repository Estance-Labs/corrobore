#![no_main]
//! Fuzz target: the persisted-record decode path.
//!
//! The audit asked for a "STIX import" fuzz target, but the codebase has no STIX
//! *import* path — `export-stix` only *exports* typed graph data to STIX (it
//! never parses untrusted STIX bytes). The real untrusted-bytes -> typed-record
//! boundary in the storage layer is the record codec: when a file-backed store
//! pages a record in, it decodes checksummed JSON-Lines bytes recovered from
//! disk. That is the ingestion surface most in need of fuzzing, so this target
//! exercises `decode_persisted_record_envelope` over arbitrary bytes.
//!
//! We recompute the checksum over the fuzz bytes so the checksum gate always
//! passes, ensuring the fuzzer reaches (and stresses) the JSON deserializer
//! rather than bouncing off checksum validation.

use graph_storage::{
    calculate_encoded_record_checksum, decode_persisted_record_envelope, EncodedRecord,
    JsonLinesRecordCodec, PersistedRecordKind, RecordFormat, StorageVersion,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let codec = JsonLinesRecordCodec::default();
    let Ok(checksum) = calculate_encoded_record_checksum(&codec, data) else {
        return;
    };
    let encoded = EncodedRecord {
        storage_version: StorageVersion::V1,
        record_format: RecordFormat::JsonLinesV1,
        kind: PersistedRecordKind::Node,
        bytes: data.to_vec(),
        checksum,
    };
    // Must return a typed Result (Ok or Err) — never panic — on arbitrary bytes.
    let _ = decode_persisted_record_envelope(&codec, &encoded, Some(PersistedRecordKind::Node));
});
