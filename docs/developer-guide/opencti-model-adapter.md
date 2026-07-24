# OpenCTI model adapter

Corrobore maps the OpenCTI `7.260722.0` graph model through the
`opencti-adapter` crate. The adapter is intentionally separate from
`graph-core`: OpenCTI-specific schemas, identifiers, access metadata, and
compatibility rules can evolve without adding CTI policy to the generic graph
engine.

The source lock, mapping contract, and golden fixtures live together:

- `compatibility/opencti/7.260722.0/source-lock.json` pins the upstream source.
- `compatibility/opencti/7.260722.0/model-mapping.json` describes mapping
  version `1.0` in a machine-readable form.
- `compatibility/opencti/7.260722.0/model-fixtures.json` covers every supported
  object and relationship family, access context, and nested extension.
- `compatibility/opencti/7.260722.0/parity-corpus.json` remains a regression
  corpus and is also required to round-trip without loss.

## Canonical graph representation

Objects become graph nodes with the `OpenCtiObject` base label, a family label,
and a sanitized entity-type label. Relationships retain their original
direction, semantic type, source identifier, and target identifier.

Every canonical graph record stores:

| Property | Meaning |
| --- | --- |
| `opencti.raw` | The complete typed JSON record, including unknown fields and nested extensions |
| `opencti.mapping_version` | Independent adapter mapping version |
| `opencti.family` | Pinned object or relationship family |
| `opencti.canonical_id` | Stable OpenCTI identity used by the projection |
| `opencti.identifiers` | Typed identifier keys |
| `opencti.references` | Reference-bearing fields |
| `opencti.access` | Access-policy inputs, without authorization decisions |
| `opencti.timestamps` | Validated semantic timestamps |
| `opencti.provenance` | External references, source references, and migration metadata |
| `opencti.field.*` | Typed copies of top-level fields for graph consumers |

The lossless `opencti.raw` value is authoritative for reconstruction. Typed
copies are projections and must not be used to discard fields from the raw
record.

## Families and forward compatibility

The adapter distinguishes STIX domain objects, cyber observables, meta objects,
OpenCTI internal objects, STIX core relationships, reference relationships,
sightings, and internal relationships.

An unknown object or relationship type is preserved as a generic typed OpenCTI
record. It is never silently converted to another semantic type such as
`Identity`. A malformed record with no usable type, identifier, relationship
type, or endpoint is rejected with an explicit mapping error.

## Identifier projection

Lookups are namespaced by identifier kind:

- internal ID;
- standard ID;
- current or historical STIX ID;
- external-reference ID;
- alias;
- deduplication ID.

One active record may own a typed identifier. Create, update, merge, delete,
replay, and rebuild operations are atomic. Updates require the exact current
revision. Deletes tombstone the record and release its identifiers. Merges move
identifiers to the survivor and tombstone sources. Replaying an identical
transaction is idempotent; reusing its transaction ID with different content
is an explicit conflict.

## Mapping migrations

The mapping version is independent of both the Corrobore storage schema and the
OpenCTI source release. A migration rebuilds identifier projections from
canonical records under the target mapping version. Conflicts are deterministic
and abort the rebuild instead of selecting a winner implicitly.

Access metadata is descriptive adapter output. Authorization and tenant-policy
decisions remain outside this crate and outside the scope of the mapping.

## Snapshot and mutation synchronization

`OpenCtiSynchronizer` applies a consistent snapshot followed by an ordered
catch-up stream. Every mutation carries a stable replay identity and monotonic
source sequence. A durable checkpoint records the snapshot phase, high-water
mark, last contiguous acknowledgement, retry queue depth, bounded replay
fingerprints, and bounded dead-letter diagnostics.

The supported mutation classes are object upsert/delete, relationship
upsert/delete, and access-policy replacement for either record kind. Upserts
replace the complete canonical payload so fields removed upstream do not remain
as stale graph properties. Relationship replacements also move adjacency
entries when endpoints change.

The HTTP runtime commits each accepted source batch as one canonical WAL
transaction, then fsyncs and atomically renames the synchronization checkpoint.
If the response or checkpoint write is lost after the graph commit, the same
batch can be replayed without duplicating canonical records.

Parity validation compares active records, lossless properties, identifier
projections, relationship endpoints and types, access-policy inputs, and
derived-projection freshness. Shadow reads remain disabled until every
dimension matches.
