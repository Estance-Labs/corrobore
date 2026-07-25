# Deployment Modes

Corrobore exposes one graph runtime through two process models: embed the engine
inside a Rust application, or run it as a separate service. The HTTP API is the
network contract of the service model, while the standalone executable is its
supported operational entry point.

## Choose a mode

| Mode | Process boundary | Interface | Choose it when |
| :--- | :--- | :--- | :--- |
| **Embedded Engine** | Inside the host Rust process | `corrobore_engine` Rust API | A Rust application should own engine construction, policy, lifecycle, and direct calls without a network service. |
| **HTTP Server** | Separate server process | Authenticated HTTP/JSON API | You are integrating an agent, service, Python client, or other remote caller and need the route, authentication, limit, and response contracts. |
| **Standalone Server** | Separate `corrobore` process | HTTP/JSON plus operator CLI | You are deploying Corrobore as a durable service and need validated configuration, lifecycle commands, storage ownership, TLS, monitoring, backup, and upgrades. |

All three paths share the same policy, budget, validation, query-planning, and
execution layers. The choice changes who owns the process and how callers reach
the runtime; it does not create a different Cypher implementation.

## How HTTP and standalone relate

HTTP and standalone describe different layers of the same service deployment:

- the **HTTP Server** guide is the client and transport reference. Use it to
  understand routes, authentication, request limits, sessions, and response
  behavior;
- the **Standalone Server** guides are the operator reference. Use them to
  install, configure, start, supervise, persist, secure, back up, and upgrade
  the process that exposes that HTTP API.

The unified `corrobore server start` command is the supported standalone product
entry point. The legacy `corrobore-http-server` binary remains a development
compatibility target; it is not a second production deployment mode.

The **Embedded Engine** does not open an HTTP listener. The host application
owns its lifecycle and calls the Rust facade directly. Choose the service model
when you need language-neutral or remote access, centralized authentication, or
operator-managed durability.

## Detailed guides

- [Embedded Engine](embedded-engine.md) — Rust facade, identity, policy, seed
  search, and deterministic export.
- [HTTP Server](http-server.md) — HTTP routes, authentication, limits, sessions,
  imports, exports, and error contracts.
- [Standalone Server CLI](standalone-server.md) — unified commands, lifecycle,
  security boundaries, and distribution entry point.
- [Standalone Configuration](standalone-configuration.md) — precedence, TOML
  fields, environment variables, CLI overrides, and defaults.
- [Standalone Operations](standalone-operations.md) — native, Docker, systemd,
  observability, backup, restore, upgrade, and rollback runbooks.

For a first local run, continue with [Getting Started](../getting-started.md).
For container deployment, use
[Getting Started with Docker](../getting-started-docker.md).
