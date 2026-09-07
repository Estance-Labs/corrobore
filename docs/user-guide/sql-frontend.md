# SQL Frontend

Corrobore can answer SQL over the PostgreSQL wire protocol through an opt-in
listener. A PostgreSQL client or driver connects to `postgres://host:5432`,
authenticates with the Corrobore bearer token as its password, and queries a
documented relational projection of the graph. SQL is an alternative query
language to Cypher, not a second engine: both frontends compile into the same
structural query AST, and the same planner, executor, policies, budgets and
durability path run it.

Accepting a PostgreSQL connection does not make Corrobore a PostgreSQL server.
The supported dialect is the subset below, the transaction model is
Corrobore's, and the boundaries listed at the end are deliberate.

## Enable the listener

The listener is off by default. Add `sql` to the enabled interfaces and choose
a port distinct from the HTTP and Bolt ports:

```toml
[server]
host = "127.0.0.1"
port = 8080
auth_token_file = "/etc/corrobore/secrets/http-token"

[interfaces]
enabled = ["http", "sql"]

[sql]
port = 5432
max_connections = 64
```

The same settings are available as `CORROBORE_SERVER_INTERFACES`,
`CORROBORE_SQL_PORT` and `CORROBORE_SQL_MAX_CONNECTIONS`, or as
`--interfaces http,sql --sql-port 5432 --sql-max-connections 64`. The listener
binds `server.host`, so the [network exposure rules](standalone-server.md)
apply unchanged. When `[tls]` is enabled the listener answers `SSLRequest`
with `S` and upgrades the connection with the same certificate and key
(`sslmode=require` works); without TLS it answers `N`.

`corrobore server validate-config --print-effective` prints `sql.port` and
`sql.max_connections`. The legacy `corrobore-http-server` binary does not open
a SQL listener.

## Connect

Authentication is cleartext password over the wire (use TLS beyond loopback):
the password is the Corrobore bearer token, the user name is recorded but not
checked, and the database is always `corrobore`.

```javascript
import pg from "pg";

const client = new pg.Client({
  host: "127.0.0.1", port: 5432, database: "corrobore",
  user: "analyst", password: process.env.CORROBORE_HTTP_AUTH_TOKEN,
});
await client.connect();
const { rows } = await client.query(
  'SELECT a.name, m.name AS malware FROM "ThreatActor" AS a JOIN "USES" AS r ON r.source_id = a.id JOIN "Malware" AS m ON r.target_id = m.id LIMIT 20',
);
```

```bash
PGPASSWORD="$CORROBORE_HTTP_AUTH_TOKEN" psql "host=127.0.0.1 port=5432 dbname=corrobore user=analyst" \
  -c 'SELECT table_name FROM information_schema.tables WHERE table_schema = '"'"'public'"'"' ORDER BY table_name'
```

In `local-insecure` authentication mode on loopback no password round happens.

## Relational projection

| Table | Rows | Columns |
| :--- | :--- | :--- |
| `"<Label>"` (quote it to keep the case) | Nodes carrying that label | `id`, `labels` (jsonb), `status`, `confidence`, `evidence_refs` (jsonb), `properties` (jsonb), and every property as its own column |
| `nodes` | Every node | The same columns |
| `relationships` | Every relationship | `id`, `type`, `source_id`, `target_id`, `status`, `confidence`, `evidence_refs`, `properties`, and every property |
| `"<TYPE>"` in a join | Relationships of that type | The relationship columns |

A relationship table is recognised by how it is joined:

```sql
SELECT a.name, r.type, m.name AS malware
FROM "ThreatActor" AS a
JOIN "USES" AS r ON r.source_id = a.id
JOIN "Malware" AS m ON r.target_id = m.id
WHERE m.family = 'X-Agent'
ORDER BY a.name LIMIT 50;
```

compiles to the bounded pattern `(a:ThreatActor)-[r:USES]->(m:Malware)`, the
one traversal shape the shared AST carries. Either endpoint may be left out of
the joins; `FROM relationships AS r WHERE r.type = 'USES'` folds the type into
the pattern. Cross joins, node-to-node joins without a relationship, outer
joins and more than one relationship per statement are refused with
`0A000`.

`information_schema.tables` and `information_schema.columns` are answered from
the labels and properties the graph currently holds, with property types
inferred from values (`bigint`, `double precision`, `boolean`, `text`,
`jsonb`). `SELECT version()`, `current_user`, `current_database()` and the
`SET`, `SHOW` and transaction statements clients send at connect are handled
locally.

## SQL coverage

| Statement | Supported forms |
| :--- | :--- |
| `SELECT` | `[DISTINCT]` columns with aliases, `*`, `count(*)`, `count(alias)`, `sum`, `avg`, `min`, `max`; `FROM` one table plus relationship joins; `WHERE` with `=`, `<>`, `<`, `<=`, `>`, `>=`, `IN`, `NOT IN`, `IS [NOT] NULL`, `AND`, `OR`, parentheses; `ORDER BY` columns; `LIMIT`; `OFFSET`; positional `$1` and named `$name` parameters; `::type` casts are accepted and ignored |
| `INSERT` | `INSERT INTO "Label" (cols) VALUES (...) [RETURNING cols | *]`, one row per statement; `ARRAY[...]` literals |
| `UPDATE` | `UPDATE "Label" [AS x] SET col = literal, ... [WHERE ...]` |
| `DELETE` | `DELETE FROM "Label" [AS x] [WHERE ...]` |
| Transactions | `BEGIN` / `START TRANSACTION` (`READ ONLY` honoured, isolation options accepted), `COMMIT` / `END`, `ROLLBACK` / `ABORT` |
| Session | `SET`, `RESET`, `DISCARD` (accepted, no effect), `SHOW <setting>` for the settings clients probe |

Predicates and `ORDER BY` work on properties and on `id`, `status` and
`confidence`. Structural columns (`labels`, `type` beyond the equality fold,
`source_id`, `target_id`, `properties`) can be selected but not filtered or
ordered on; aggregates cannot be mixed with plain columns (`42803`) because
`GROUP BY` is not supported. Mutations run through the same policy, budget and
durability path as Cypher: a read-only runtime refuses them with `42501`.

Values are typed. Integers arrive as `int8`, decimals as `float8`, booleans as
`bool`, strings as `text`, lists, maps and whole records as `jsonb`. Column
types are described from the rows a statement actually returned; a column that
is NULL in every row, or a statement described before it runs, is typed from
the catalog. Parameters bound with an unspecified type are read the way a
literal would be: `42` is an integer, `true` a boolean, anything else text.
Binary-format parameters are accepted for `bool`, `int2`, `int4`, `int8`,
`float4`, `float8` and `text`; results are always text format.

## Protocol coverage

Startup with `SSLRequest`, `GSSENCRequest` (declined) and cancel requests
(ignored); cleartext password authentication; the parameter statuses drivers
expect (`server_version`, `client_encoding`, `standard_conforming_strings`,
`integer_datetimes`, ...); `BackendKeyData`; the simple query protocol with
several statements per message; the extended protocol (`Parse`, `Bind`,
`Describe` of statements and portals, `Execute` with a row limit and
`PortalSuspended`, `Close`, `Flush`, `Sync`); `Terminate`. `COPY` and the
fast-path function interface are refused with `08P01`.

Each statement runs under the same request timeout as the HTTP routes and is
bounded by the runtime's returned-record and payload budgets; rows are written
as `DataRow` messages from that bounded buffer. Connections beyond
`sql.max_connections` wait for a slot.

## Transaction model

Corrobore applies each statement atomically and durably on its own, so a SQL
transaction is a **statement group**:

- `BEGIN` opens a group; `ReadyForQuery` reports `T` while it is open;
- statements are applied as they run, each through the engine's atomic path;
- an error fails the group: further statements answer `25P02` until `COMMIT`
  (which then answers `ROLLBACK`, as PostgreSQL does) or `ROLLBACK`;
- `COMMIT` acknowledges; `ROLLBACK` of a group that only read is a no-op;
- `ROLLBACK` after a statement that wrote **ends the group but does not revert
  the write**. The client receives `ROLLBACK` together with a `WARNING`
  notice (`01000`) saying the statements were already applied, because a
  PostgreSQL client must always be able to leave a transaction and pretending
  the write vanished would be worse.

`BEGIN READ ONLY` refuses writes with `42501`. Isolation levels are accepted
and mean nothing beyond that.

## Errors

Errors carry the standard SQLSTATE a client acts on and a message that never
contains the token, storage paths or configuration.

| SQLSTATE | When |
| :--- | :--- |
| `42601` syntax_error | SQL the parser rejects, and statements the bounded executor refuses |
| `0A000` feature_not_supported | Recursive CTEs, `GROUP BY`, outer or cross joins, relationship inserts, `LIKE`, subqueries, DDL, `COPY` |
| `42P01` undefined_table | An alias no `FROM` item declares |
| `42703` undefined_column | Inserting or updating a derived structural column |
| `42702` ambiguous_column | An unqualified column with several tables |
| `42803` grouping_error | Aggregates mixed with plain columns |
| `42501` insufficient_privilege | A write in a read-only group, or refused by the runtime policy |
| `22023` invalid_parameter_value | A `$n` with no bound value |
| `25P02` in_failed_sql_transaction | A statement after an error in an open group |
| `54000` program_limit_exceeded | A runtime budget or request limit stopped the statement |
| `57014` query_canceled | The request timeout |
| `57P01` admin_shutdown | The server is draining |
| `28P01` invalid_password | Authentication failed (the connection closes) |
| `08P01` protocol_violation | Malformed or unsupported protocol messages, parameter count mismatch |
| `XX000` internal_error | The engine could not execute the statement; the cause is in the server log |

## Known incompatibilities

- No recursive common table expressions or variable-length traversal: the
  shared AST carries one relationship hop per statement.
- No `GROUP BY`, `HAVING`, set operations, subqueries, `LIKE`, `BETWEEN`,
  `NOT`, arithmetic in expressions, or functions beyond the connect-time set.
- Relationship inserts, multi-row `VALUES`, `INSERT ... SELECT`, `UPDATE ...
  FROM`, `DELETE ... USING`, `ON CONFLICT` and `RETURNING` on `UPDATE` and
  `DELETE` are refused; use Cypher for those shapes.
- Unquoted identifiers keep their case instead of folding to lower case; quote
  labels (`"ThreatActor"`).
- Rollback of applied writes is refused in effect (see the transaction model);
  isolation levels are accepted without meaning.
- `pg_catalog` queries a tool may issue beyond `information_schema` and the
  documented `SHOW` settings are not answered; `psql` meta-commands that rely on
  them (`\d`) do not work.
- One database, `corrobore`; one schema, `public`.
- No idle-connection timeout is enforced; the connection cap bounds resources.

## Verification

The protocol contract is
`crates/corrobore-http-server/tests/sql_wire_contract.rs`: a client written
from the protocol specification covers startup and authentication,
`SSLRequest` handling, scalar and catalog statements, a CRUD round trip with
typed columns, joins and aggregates, the extended protocol with text and
binary parameters and portal suspension, SQLSTATE mapping, statement-group
transactions, shutdown behaviour, and the standalone `sql` interface and
configuration. The compiler contract is
`crates/sql-frontend/tests/compile_contract.rs`.

`scripts/sql-driver-conformance.mjs` runs the maintained `pg` (node-postgres)
driver against a live listener (`CORROBORE_SQL_URL`, `CORROBORE_SQL_TOKEN`,
and `CORROBORE_HTTP_URL` for the Cypher relationship fixture). Recorded result
for this item: `pg` 8.x connected with password authentication to
`corrobore server start --interfaces http,sql` and passed all six checks
(version, typed CRUD with parameters, `information_schema`, a relationship join
with aggregates, statement-group transactions and SQLSTATEs, refused
credentials) on 2026-09-07 (macOS, Node.js 26). `psql` itself was not
available where this item was verified; the wire contract covers the simple
query protocol it uses. See
[Deployment Modes](deployment-modes.md) for how the listener relates to the
HTTP, Bolt and standalone modes.
