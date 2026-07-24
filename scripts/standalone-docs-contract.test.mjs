import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../", import.meta.url);

async function read(path) {
  return readFile(new URL(path, root), "utf8");
}

function rustStructFields(source, name) {
  const body = source.match(new RegExp(`struct ${name} \\{([\\s\\S]*?)\\n\\}`))?.[1];
  assert.ok(body, `Rust struct ${name} must exist`);

  return [...body.matchAll(/^    ([a-z][a-z0-9_]*):/gm)].map((match) => match[1]);
}

function environmentVariables(...sources) {
  const names = new Set();
  for (const source of sources) {
    for (const match of source.matchAll(/"(CORROBORE_[A-Z0-9_]+)"/g)) {
      names.add(match[1]);
    }
  }
  return [...names].sort();
}

function cliFlags(source) {
  return [
    ...new Set(
      [...source.matchAll(/#\[arg\(long[^\]]*\)\]\s+([a-z][a-z0-9_]*):/g)].map(
        (match) => `--${match[1].replaceAll("_", "-")}`,
      ),
    ),
  ].sort();
}

test("standalone reference covers every command CLI flag TOML field and environment variable", async () => {
  const [cliSource, configSource, reference] = await Promise.all([
    read("crates/corrobore-http-server/src/bin/corrobore.rs"),
    read("crates/corrobore-http-server/src/config.rs"),
    read("docs/user-guide/standalone-configuration.md"),
  ]);

  for (const command of ["start", "validate-config", "status", "version"]) {
    assert.match(reference, new RegExp(`corrobore server ${command}`));
  }

  for (const flag of cliFlags(cliSource)) {
    assert.ok(reference.includes(`\`${flag}\``), `missing CLI flag ${flag}`);
  }

  const sections = new Map([
    ["server", "FileServer"],
    ["storage", "FileStorage"],
    ["logging", "FileLogging"],
    ["limits", "FileLimits"],
    ["interfaces", "FileInterfaces"],
    ["maintenance", "FileMaintenance"],
    ["operations", "FileOperations"],
    ["tls", "FileTls"],
  ]);
  for (const [section, structName] of sections) {
    for (const field of rustStructFields(cliSource, structName)) {
      const path = `${section}.${field}`;
      assert.ok(reference.includes(`\`${path}\``), `missing TOML field ${path}`);
    }
  }

  for (const variable of environmentVariables(cliSource, configSource)) {
    assert.ok(reference.includes(`\`${variable}\``), `missing environment variable ${variable}`);
  }

  assert.match(
    reference,
    /CLI arguments > environment variables > TOML file > defaults/,
  );
});

test("operator guide contains executable native Docker systemd and operational examples", async () => {
  const [guide, service, systemdConfig, dockerfile] = await Promise.all([
    read("docs/user-guide/standalone-operations.md"),
    read("packaging/systemd/corrobore.service"),
    read("packaging/systemd/corrobore.toml"),
    read("Dockerfile"),
  ]);

  for (const expected of [
    "corrobore server validate-config --config /etc/corrobore/corrobore.toml",
    "corrobore server start --config /etc/corrobore/corrobore.toml",
    "docker compose up --wait",
    "systemctl enable --now corrobore.service",
    "systemctl stop corrobore.service",
    "GET /health/live",
    "GET /health/ready",
    "GET /version",
    "GET /metrics",
  ]) {
    assert.ok(guide.includes(expected), `operator guide should include ${expected}`);
  }

  assert.ok(
    service.includes(
      "ExecStart=/usr/local/bin/corrobore server start --config /etc/corrobore/corrobore.toml",
    ),
  );
  for (const path of [
    "/var/lib/corrobore/runtime",
    "/var/lib/corrobore/graph",
    "/var/log/corrobore",
  ]) {
    assert.ok(systemdConfig.includes(path), `systemd configuration should include ${path}`);
  }
  assert.ok(
    dockerfile.includes(
      'CMD ["server", "start", "--config", "/etc/corrobore/corrobore.toml"]',
    ),
  );
});

test("backup restore upgrade and rollback procedures are explicit and tied to validation", async () => {
  const [guide, backupTests] = await Promise.all([
    read("docs/user-guide/standalone-operations.md"),
    read("crates/graph-storage/tests/backup_restore_integrity.rs"),
  ]);

  for (const heading of [
    "## Backup",
    "## Restore",
    "## Upgrade",
    "## Rollback",
  ]) {
    assert.ok(guide.includes(heading), `operator guide should include ${heading}`);
  }

  for (const expected of [
    "consistent offline backup",
    "empty restore target",
    "manifest.json",
    "storage_compatibility",
    "active_storage_version",
    "active_record_format",
    "cargo test -p graph-storage --test backup_restore_integrity --locked",
  ]) {
    assert.ok(guide.includes(expected), `operator guide should include ${expected}`);
  }

  for (const testName of [
    "backup_restore_roundtrip_is_semantically_equivalent_to_source_checkpoint",
    "backup_validation_reports_corruption_explicitly",
    "restore_requires_empty_target_root",
  ]) {
    assert.ok(backupTests.includes(testName), `backup suite should include ${testName}`);
  }
});

test("new standalone guides are part of the published navigation", async () => {
  const navigation = await read("mkdocs.yml");

  assert.ok(navigation.includes("user-guide/standalone-configuration.md"));
  assert.ok(navigation.includes("user-guide/standalone-operations.md"));
});
