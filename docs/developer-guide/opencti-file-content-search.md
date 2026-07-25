# OpenCTI file-content search

Issue #48 implements the `file.search`, `file.index`, `file.delete`, and
`file.extract` compatibility boundary without making Corrobore the blob store.
S3/MinIO remains authoritative. A deployment exposes a read-only synchronized
view of those objects to a dedicated worker process.

## Supported formats and limits

The extraction subset is deliberately narrow:

- UTF-8 plain text;
- CSV;
- HTML/XHTML, excluding script and style content;
- PDF without encryption;
- XLS, XLSX, XLSB, and ODS spreadsheets.

Every job verifies its canonical SHA-256 digest before parsing. Source bytes,
extracted bytes, PDF pages, spreadsheet sheets, rows and cells, chunk count,
chunk length, and wall-clock execution have explicit ceilings. Unsupported,
encrypted, malformed, hash-mismatched, oversized, and exhausted inputs produce
stable failure codes and bounded diagnostics without including source content.
OCR, arbitrary document formats, antivirus scanning, and blob storage are out
of scope.

## Worker isolation and durability

The server persists deterministic jobs under
`<storage>/file-content/metadata`. The identity is derived from file ID,
content hash, and version, so replay does not create a second searchable copy.
Each lease has an unpredictable fencing token. Expired process or host-crash
leases count as timeouts, resume after the lease deadline, and eventually enter
quarantine after the configured attempt budget.

`corrobore-file-worker` is a supervisor. It launches one extractor subprocess
per polling cycle and kills it at `CORROBORE_FILE_MAX_RUNTIME_MS`. The Compose
profile additionally runs the worker as non-root with a read-only filesystem,
no network, no Linux capabilities, `no-new-privileges`, a bounded PID set,
memory/CPU ceilings, a `noexec` temporary filesystem, a read-only blob mount,
and only the extraction metadata volume writable. Cross-process job updates use
an advisory lock plus atomic fsync-and-rename publication.

Start the optional profile after creating the read-only blob source directory:

```bash
mkdir -p .corrobore-files
docker compose --profile file-search up --build --wait
```

The blob path is an adapter boundary for the deployment's existing S3/MinIO
synchronization. It is not a second authoritative copy managed by Corrobore.

## Search contract

The Knowledge Data Engine `search` expression selects file content explicitly:

```json
{
  "text": "malware.example.org",
  "content": true,
  "mode": "term",
  "mime_types": ["application/pdf", "text/plain"],
  "owner_ids": ["identity--owner"],
  "source_object_ids": ["report--source"]
}
```

Values inside one filter are alternatives; MIME type, owner, source object,
text, and authorization dimensions are combined. Results contain the canonical
file ID, record class `file_content`, snippet, highlights, MIME type, source
object ID, content hash, and version. PDF page and spreadsheet sheet/row
coordinates remain attached to extracted chunks in canonical metadata.

The shared OpenCTI access policy filters candidates before totals, ordering,
pagination, snippets, or metadata are produced. An inaccessible file therefore
cannot affect response content or counts. Cursor integrity and generation
binding reuse the full-text search contract.

## Lifecycle and rebuild

Replacement publishes only the new file version. Policy changes rewrite access
metadata without re-reading untrusted bytes. Delete removes queued versions and
searchable chunks; merge moves visibility to the surviving canonical ID.
Publication invalidates the previous derived generation first, then writes
canonical extraction artifacts and atomically publishes the rebuilt Tantivy
generation. A failed rebuild cannot serve the old generation as ready.

Deleting `search/file-content-v1/published` is recoverable: the next search or
an explicit rebuild reconstructs it from Corrobore-owned canonical extraction
metadata, whose descriptors retain the S3/MinIO key, file/object IDs, digest,
version, and access policy. Schema or content changes are re-enqueued from those
descriptors and the authoritative object source.

## Observability and compatibility gates

`/metrics` exposes low-cardinality gauges and counters for queue depth,
processing latency, failures, retries, quarantines, extracted bytes, and oldest
pending-job/index lag. Labels never contain file IDs, object IDs, names, paths,
hashes, query text, or access-policy values.

The acceptance suite covers every supported format, safe parser failures,
process isolation, crash recovery, concurrent server/worker writes, lifecycle
changes, authorization, rebuilding, snippets, provenance, and the pinned
`file-full-text` journey in
`compatibility/opencti/7.260722.0/reference-results.json`.

The reproducible release-profile benchmark indexes 100,000 synthetic file
artifacts and runs 20 warmups plus 60 measured phrase searches. The recorded
run reached 1.198 ms P95 against the 6.1608 ms parity ceiling derived from the
pinned OpenSearch full-text reference, while indexing 56,646 files/s. Exact
inputs and measurements are stored in
`compatibility/opencti/7.260722.0/file-content-benchmark-results.json`.

Reproduce it with:

```bash
cargo run --release -p opencti-file-search \
  --example small_profile_benchmark --locked
```
