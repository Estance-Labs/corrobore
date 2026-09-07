# Bolt Protocol

Corrobore can accept connections from the Neo4j driver ecosystem through an
opt-in Bolt listener. A supported driver points at `bolt://host:7687`, logs in
with the Corrobore bearer token, runs the supported Cypher subset, and receives
typed records, nodes and relationships. The listener is one more adapter over
the same engine, policies, budgets and durability path the HTTP API uses; it
grants nothing the HTTP surface does not.

Accepting a Bolt connection does not make Corrobore a Neo4j server. The
supported Cypher is [Corrobore's bounded subset](cypher.md), the transaction
model is Corrobore's, and the boundaries below are deliberate.

## Enable the listener

The listener is off by default. Add `bolt` to the enabled interfaces and choose
a port distinct from the HTTP port:

```toml
[server]
host = "127.0.0.1"
port = 8080
auth_token_file = "/etc/corrobore/secrets/http-token"

[interfaces]
enabled = ["http", "bolt"]

[bolt]
port = 7687
max_connections = 64
```

The same settings are available as `CORROBORE_SERVER_INTERFACES`,
`CORROBORE_BOLT_PORT` and `CORROBORE_BOLT_MAX_CONNECTIONS`, or as
`--interfaces http,bolt --bolt-port 7687 --bolt-max-connections 64`. The
listener binds `server.host`, so the [network exposure rules](standalone-server.md)
apply unchanged: a non-loopback bind requires TLS, required authentication and
authenticated operational endpoints. When `[tls]` is enabled the Bolt listener
terminates TLS with the same certificate and key, and drivers connect with
`bolt+s://` or `neo4j+s://`.

`corrobore server validate-config --print-effective` prints `bolt.port` and
`bolt.max_connections`. The legacy `corrobore-http-server` binary does not open
a Bolt listener.

## Connect with a driver

Authentication reuses the HTTP bearer token. Use it as the password of a basic
scheme (the principal is recorded but not checked) or as a bearer credential:

```javascript
import neo4j from "neo4j-driver";

const driver = neo4j.driver(
  "bolt://127.0.0.1:7687",
  neo4j.auth.basic("analyst", process.env.CORROBORE_HTTP_AUTH_TOKEN),
);
const { records } = await driver.executeQuery(
  "MATCH (a:ThreatActor)-[r:USES]->(m:Malware) RETURN a.name, m.name LIMIT 20",
);
```

```python
from neo4j import GraphDatabase

with GraphDatabase.driver("bolt://127.0.0.1:7687",
                          auth=("analyst", token)) as driver:
    records, summary, keys = driver.execute_query(
        "MATCH (n:Indicator) RETURN n LIMIT 10")
```

`neo4j://` URIs work too: `ROUTE` returns a routing table with this server in
every role. In `local-insecure` authentication mode on loopback, the `none`
scheme is accepted. A failed login returns
`Neo.ClientError.Security.Unauthorized` and closes the connection.

## What the wire carries

| Area | Behaviour |
| :--- | :--- |
| Versions | Bolt 4.4 and 5.0 to 5.8. The handshake picks the highest version the client offers that the server speaks. A driver that leads with the handshake-v2 manifest marker is answered from its other offers. |
| Authentication | `HELLO` with inline credentials on 4.4 and 5.0; `HELLO` then `LOGON` on 5.1+. `LOGOFF` returns to the authentication state. |
| Queries | `RUN` executes one statement; `PULL n` streams up to `n` records and reports `has_more`; `DISCARD` drops the rest. Autocommit results must be pulled or discarded before the next `RUN`. |
| Result metadata | `RUN` succeeds with the ordered `fields`; the final `SUCCESS` carries `type` (`r`, `w` or `rw`), `db`, `t_last`, write `stats`, and a `bookmark` for autocommit writes. |
| Values | Null, boolean, integer, float, string and list parameters bind to Cypher parameters. Records carry the executor's typed values: integers stay integers, floats stay floats, lists stay lists. |
| Nodes | `Node` structures with labels and properties. Native metadata is exposed under the reserved property keys `status`, `confidence` and `evidence_refs`, the same names Cypher property access uses. Bolt 5 adds the `element_id`, which is the Corrobore identifier; the integer `id` is a stable digest of it. |
| Relationships | `Relationship` structures with type, endpoints and properties; Bolt 5 adds the element identifiers of the relationship and both endpoints. |
| Transactions | `BEGIN` (with `mode: "r"` for read-only groups), `COMMIT` returning a bookmark, `ROLLBACK`; see the transaction model below. |
| Routing | `ROUTE` returns `WRITE`, `READ` and `ROUTE` entries for the address the client used, database `corrobore`, TTL 300 seconds. |
| Control | `RESET` recovers a failed session; `GOODBYE` closes; `TELEMETRY` (5.4+) is acknowledged and discarded. |

The result of each statement is buffered server-side, bounded by the runtime's
returned-record and payload budgets, and handed to the driver in the batches it
asks for. Connections beyond `bolt.max_connections` wait for a slot rather than
being refused. Each statement runs under the same request timeout as the HTTP
routes.

## Transaction model

Corrobore applies each request atomically and durably on its own. A Bolt
transaction is therefore a **statement group**, not an isolation scope:

- statements inside `BEGIN` … `COMMIT` are applied as they run, each through
  the engine's atomic mutation path;
- `COMMIT` succeeds and returns a bookmark;
- `ROLLBACK` of a group that only read succeeds;
- `ROLLBACK` after a statement that wrote is refused with
  `Neo.ClientError.Transaction.RollbackNotSupported`, because the write is
  already durable and pretending otherwise would leave the client with a wrong
  picture of the graph.

Driver transaction functions (`executeRead`, `executeWrite`, managed
transactions) work with this model; a client that relies on rolling back
applied writes does not, and should issue one statement per transaction.
Bookmarks are opaque and are not used for causal chaining.

## Failure codes

Corrobore and Cypher failures map to stable codes. Drivers act on the class
prefix: `ClientError` is final, `TransientError` may be retried,
`DatabaseError` is a server fault. Messages never include the bearer token,
storage paths or configuration.

| Code | When |
| :--- | :--- |
| `Neo.ClientError.Security.Unauthorized` | Bad or missing credentials; a request before `LOGON`. |
| `Neo.ClientError.Security.Forbidden` | A write refused by the runtime policy, for example inside a read transaction (`WRITE_PERMISSION_REQUIRED`). |
| `Neo.ClientError.Statement.SyntaxError` | Cypher the bounded parser rejects, including unsupported clauses. |
| `Neo.ClientError.Statement.ArgumentError` | A parameter of a type Corrobore cannot bind (bytes, maps, structures, non-finite floats). |
| `Neo.ClientError.Request.Invalid` | Protocol misuse: `PULL` without a result, `COMMIT` without a transaction, a message before `HELLO`, an unknown signature. |
| `Neo.ClientError.Transaction.RollbackNotSupported` | `ROLLBACK` after an applied write. |
| `Neo.ClientError.Transaction.TransactionTimedOut` | The statement exceeded the request timeout. |
| `Neo.TransientError.General.ResourceBudgetExceeded` | A runtime budget or request limit stopped the statement. |
| `Neo.TransientError.General.DatabaseUnavailable` | The server is draining for shutdown. |
| `Neo.DatabaseError.Statement.ExecutionFailed` | The engine could not execute the statement; the cause is in the server log. |

Every `FAILURE` carries `code` and `message`. From Bolt 5.7 the message also
carries the GQL-era fields drivers read there: `neo4j_code`, `gql_status`
(`42000` for statement errors, `42N42` for security errors, `50N42`
otherwise), `status_description`, `description` and a default
`diagnostic_record`.

After a `FAILURE`, further requests are `IGNORED` until the driver sends
`RESET`, as the Bolt specification requires. Protocol violations close the
connection after the `FAILURE`.

## Known incompatibilities

- Only the [bounded Cypher subset](cypher.md) is available; there are no paths,
  variable-length patterns, `UNWIND`, procedures, or temporal and spatial
  values. Unsupported queries fail with a syntax-class code.
- One database, `corrobore`; the `db` selector is accepted and ignored.
- Rollback of applied writes is refused; see the transaction model.
- Bookmarks and transaction metadata are accepted but carry no causal
  consistency semantics.
- The handshake-v2 manifest negotiation is not implemented; drivers fall back
  to the classic negotiation the specification describes.
- Notification and telemetry configuration is acknowledged and not acted on.
- No idle-connection timeout is enforced; the connection cap bounds resources.

## Verification

The protocol contract is `crates/corrobore-http-server/tests/bolt_protocol_contract.rs`:
a client written from the specification covers negotiation, PackStream
framing, both authentication flows, typed records and structures in Bolt 4.4
and 5, batched streaming, failure codes and `RESET` recovery, statement-group
transactions, routing, shutdown behaviour, and the standalone `bolt` interface
and configuration. PackStream and negotiation have unit tests beside their
code.

`scripts/bolt-driver-conformance.mjs` runs the official `neo4j-driver` for
Node.js against a live listener (`CORROBORE_BOLT_URL`, `CORROBORE_BOLT_TOKEN`)
and exercises `executeQuery`, managed read and write transactions, typed
values, node and relationship structures, streaming with a small fetch size,
and the failure path. Recorded result for this item: `neo4j-driver` 6.2.0
negotiated Bolt 5.8 against `corrobore server start --interfaces http,bolt`
and passed all eight checks over both `bolt://` and `neo4j://`
(2026-09-07, macOS, Node.js 26). The script needs the driver package and a
running server; it is not part of the automated suite. `psql`-style tooling
is out of scope here; see the SQL frontend for PostgreSQL clients. See
[Deployment Modes](deployment-modes.md) for how the listener relates to the
HTTP and standalone modes.
