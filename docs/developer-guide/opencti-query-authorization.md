# OpenCTI query authorization

Issue #45 enforces OpenCTI authorization on the fundamental Knowledge Data
read surface. The policy is compiled once per request and is shared by the
embedded engine and the persistent candidate-selection layer.

## Policy inputs

The caller supplies an `AccessContext` containing the subject, organizations,
markings, tenant, roles and policy attributes. `policy_version` identifies the
authorization snapshot. Optional comma-separated policy attributes provide
`member_ids`, `authority_ids` and `sharing_grants`.

Every mapped node and relationship carries a payload-free `opencti.access`
document. It can contain marking, organization and tenant restrictions,
authorized-member objects, creator and owner identifiers, authority
identifiers, and sharing-policy entries.

The compiler rejects an empty non-system subject and normalizes all sets before
computing a stable access fingerprint. Malformed access metadata fails closed.

## Decision order

System-role callers bypass record restrictions. For all other callers:

1. every required marking and tenant constraint must match;
2. an explicit sharing denial rejects the record;
3. owner, creator, authorized-member, authority, explicit sharing grant or
   organization membership may grant access;
4. a record without an identity restriction is allowed after the hard
   constraints pass.

Authorized-member entries grant only through recognized identifier fields and
never through a false or deny-valued permission. Marking and tenant constraints
remain mandatory even when an identity exception matches.

## Candidate pushdown

The persistent catalog stores compact access documents for current nodes and
relationships. Point, filtered, ordered, counted and paginated reads intersect
these indexes before invoking the payload pager. Adjacency expansion evaluates
the relationship and the neighbor before degree checks or page-in. Generic
relationship selection also requires both endpoints to be accessible.

Consequently, inaccessible records do not enter hot or warm resident sets and
do not influence counts, ordering, page boundaries, cursors, facets, graph
paths or supernode decisions. Stores whose older derived catalog lacks an
access document fail closed for non-system callers; rebuilding the derived
indexes restores those documents from committed mutation journals.

## Policy changes and cursors

The resident payload cache is scoped by the normalized access fingerprint. A
different subject, organization, marking, tenant, role, policy version or
policy attribute clears resident nodes and relationships before selection.

Pagination tokens are bound to both `policy_version` and the access
fingerprint. The token carries only a pagination-keyed HMAC binding, not the
context fingerprint itself. Reusing a token after a policy change returns
`STALE_PAGINATION_TOKEN`; the engine never continues from a boundary computed
under another policy.

## Non-inference behavior

A denied point lookup has the same provider response as a missing identifier:
an empty record. At the persistent boundary both cases avoid payload page-in
and use the `visibility_miss` timing class. Collection and graph reads omit
denied candidates without exposing their identifiers, relationship topology or
properties.

This contract reduces observable differences but does not claim constant-time
cryptographic execution. The adversarial storage test compares 128 denied and
missing probes and requires the slower P95 to remain within six times the
faster P95 plus two milliseconds of scheduling tolerance. Operators should
also compare bounded latency distributions in their deployment environment.

## Audit and shadow enforcement

The engine records bounded authorization audit events with correlation ID,
operation, policy version, allow/deny outcome and decision reason. Events never
contain the record identifier, access document or inaccessible payload. Paged
execution also records the aggregate number of candidates rejected before
page-in.
The HTTP shadow path emits the same payload-free fields as a structured
`opencti_authorization_decision` event.

When an access policy is present, any shadow difference in results, counts,
ordering, cursors, paths, errors or authorization outcome is classified as
`authorization_result_mismatch`. This is a security divergence and cannot be
accepted through a baseline.

## Validation

The `opencti_authorization` suite covers markings, tenants, organizations,
members, creator/owner exceptions, authorities, sharing grants and denials,
point-read non-inference, relationship-complete paths, authorized-only
ordering/count/pagination, policy-bound cursors, redacted audits and blocking
shadow divergences.

The `opencti_access_pushdown` suite verifies pre-page-in filtering, relationship
and endpoint filtering, policy-scoped cache invalidation, recovery of access
indexes, indistinguishable missing/denied storage behavior and access-metadata
mutation.
