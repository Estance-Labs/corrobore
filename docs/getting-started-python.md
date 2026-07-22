# Getting Started with Python

Corrobore exposes a standard HTTP/JSON API. Any HTTP client works — this guide uses [`requests`](https://requests.readthedocs.io/) for simplicity and [`httpx`](https://www.python-httpx.org/) for async workflows.

## Prerequisites

- Python 3.9 or later
- A running Corrobore server (see [Getting Started](getting-started.md) or [Docker](getting-started-docker.md))

## Install the HTTP client

```bash
pip install requests        # synchronous
pip install httpx           # async (optional)
```

## Synchronous client

### Minimal setup

```python
import requests

CORROBORE_URL = "http://127.0.0.1:8080"
TOKEN = "change-me"

session = requests.Session()
session.headers.update({"Authorization": f"Bearer {TOKEN}"})

# Verify the server is reachable
resp = session.get(f"{CORROBORE_URL}/health")
resp.raise_for_status()
print(resp.json()["status"])  # "ok"
```

### Write and read nodes

```python
# Write a node
resp = session.post(
    f"{CORROBORE_URL}/v1/cypher/write",
    json={"query": "MERGE (n:Indicator {name: 'phishing.example'}) RETURN n"},
)
resp.raise_for_status()
print(resp.json())

# Read nodes
resp = session.post(
    f"{CORROBORE_URL}/v1/cypher/read",
    json={"query": "MATCH (n:Indicator) RETURN n LIMIT 10"},
)
resp.raise_for_status()
records = resp.json()["result"]["records"]
for row in records:
    print(row)
```

### Session-scoped workflow

Sessions group writes and reads under a single audit trail. Use them when you need to track which agent produced which data.

```python
def open_session(session, base_url, workspace_id, actor_id, actor_kind="agent"):
    resp = session.post(
        f"{base_url}/v1/sessions/start",
        json={
            "workspace_id": workspace_id,
            "actor_id": actor_id,
            "actor_kind": actor_kind,
        },
    )
    resp.raise_for_status()
    return resp.json()["result"]["session_id"]


def close_session(session, base_url, session_id):
    resp = session.post(f"{base_url}/v1/sessions/{session_id}/stop")
    resp.raise_for_status()


# Usage
sid = open_session(session, CORROBORE_URL, "workspace-1", "analyst-bot")

session.post(
    f"{CORROBORE_URL}/v1/cypher/write",
    json={
        "query": "MERGE (n:Indicator {name: 'beacon.example'}) RETURN n",
        "session_id": sid,
    },
).raise_for_status()

logs_resp = session.get(
    f"{CORROBORE_URL}/v1/sessions/{sid}/logs",
    params={"limit": 50},
)
print(logs_resp.json())

close_session(session, CORROBORE_URL, sid)
```

### Reusable client class

```python
from __future__ import annotations

import requests


class CorroboreClient:
    def __init__(self, base_url: str, token: str, timeout: float = 10.0):
        self._base = base_url.rstrip("/")
        self._timeout = timeout
        self._http = requests.Session()
        self._http.headers.update({"Authorization": f"Bearer {token}"})

    # ------------------------------------------------------------------ #
    # Graph                                                                 #
    # ------------------------------------------------------------------ #

    def read(self, query: str, session_id: str | None = None) -> list[dict]:
        body: dict = {"query": query}
        if session_id:
            body["session_id"] = session_id
        resp = self._http.post(
            f"{self._base}/v1/cypher/read", json=body, timeout=self._timeout
        )
        resp.raise_for_status()
        return resp.json()["result"]["records"]

    def write(self, query: str, session_id: str | None = None) -> dict:
        body: dict = {"query": query}
        if session_id:
            body["session_id"] = session_id
        resp = self._http.post(
            f"{self._base}/v1/cypher/write", json=body, timeout=self._timeout
        )
        resp.raise_for_status()
        return resp.json()["result"]

    # ------------------------------------------------------------------ #
    # Sessions                                                              #
    # ------------------------------------------------------------------ #

    def start_session(
        self,
        workspace_id: str,
        actor_id: str,
        actor_kind: str = "agent",
    ) -> str:
        resp = self._http.post(
            f"{self._base}/v1/sessions/start",
            json={
                "workspace_id": workspace_id,
                "actor_id": actor_id,
                "actor_kind": actor_kind,
            },
            timeout=self._timeout,
        )
        resp.raise_for_status()
        return resp.json()["result"]["session_id"]

    def stop_session(self, session_id: str) -> None:
        self._http.post(
            f"{self._base}/v1/sessions/{session_id}/stop",
            timeout=self._timeout,
        ).raise_for_status()

    def session_logs(self, session_id: str, limit: int = 100) -> list[dict]:
        resp = self._http.get(
            f"{self._base}/v1/sessions/{session_id}/logs",
            params={"limit": limit},
            timeout=self._timeout,
        )
        resp.raise_for_status()
        return resp.json()["result"]

    def close(self) -> None:
        self._http.close()

    def __enter__(self) -> "CorroboreClient":
        return self

    def __exit__(self, *_) -> None:
        self.close()
```

**Using the client:**

```python
with CorroboreClient("http://127.0.0.1:8080", token="change-me") as corrobore:
    sid = corrobore.start_session("workspace-1", "analyst-bot")

    corrobore.write(
        "MERGE (n:Indicator {name: 'c2.example'}) RETURN n",
        session_id=sid,
    )
    rows = corrobore.read(
        "MATCH (n:Indicator) RETURN n.name AS name",
        session_id=sid,
    )
    for row in rows:
        print(row["name"])

    corrobore.stop_session(sid)
```

## Async client with httpx

```python
import asyncio
import httpx


class AsyncCorroboreClient:
    def __init__(self, base_url: str, token: str, timeout: float = 10.0):
        self._base = base_url.rstrip("/")
        self._http = httpx.AsyncClient(
            headers={"Authorization": f"Bearer {token}"},
            timeout=timeout,
        )

    async def read(self, query: str, session_id: str | None = None) -> list[dict]:
        body: dict = {"query": query}
        if session_id:
            body["session_id"] = session_id
        resp = await self._http.post(f"{self._base}/v1/cypher/read", json=body)
        resp.raise_for_status()
        return resp.json()["result"]["records"]

    async def write(self, query: str, session_id: str | None = None) -> dict:
        body: dict = {"query": query}
        if session_id:
            body["session_id"] = session_id
        resp = await self._http.post(f"{self._base}/v1/cypher/write", json=body)
        resp.raise_for_status()
        return resp.json()["result"]

    async def aclose(self) -> None:
        await self._http.aclose()

    async def __aenter__(self) -> "AsyncCorroboreClient":
        return self

    async def __aexit__(self, *_) -> None:
        await self.aclose()


async def main():
    async with AsyncCorroboreClient("http://127.0.0.1:8080", token="change-me") as corrobore:
        await corrobore.write("MERGE (n:Indicator {name: 'async.example'}) RETURN n")
        rows = await corrobore.read("MATCH (n:Indicator) RETURN n.name AS name")
        for row in rows:
            print(row["name"])


asyncio.run(main())
```

## Error handling

The API returns errors as `{ "ok": false, "error": { "code": "...", "message": "..." } }`. HTTP status codes are meaningful: `401` (invalid token), `429` (rate limit), `413` (body too large), `504` (query timeout).

```python
from requests import HTTPError

try:
    corrobore.write("INVALID CYPHER @@@@")
except HTTPError as exc:
    if exc.response is not None:
        error = exc.response.json().get("error", {})
        print(error.get("code"), error.get("message"))
```

## Next steps

- [HTTP Server reference](user-guide/http-server.md) — full route catalogue and all configuration variables.
- [Cypher support](user-guide/cypher.md) — which Cypher clauses and functions are available.
- [Intelligence Domains](user-guide/domains.md) — built-in node schemas for CTI, FIMI, and Crisis.
- [Exporters & Validation](user-guide/exporters.md) — export graph data to STIX 2.1 or FIMI format.
