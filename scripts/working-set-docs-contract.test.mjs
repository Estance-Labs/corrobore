import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../", import.meta.url);

async function read(path) {
  return readFile(new URL(path, root), "utf8");
}

test("Epic 0017 documentation reflects completed acceptance evidence", async () => {
  const [guide, architecture] = await Promise.all([
    read("docs/user-guide/working-set.md"),
    read("docs/architecture.md"),
  ]);

  // Keep the public status anchored to the migrated epic and acceptance
  // trackers, the executable acceptance suite, and the committed report.
  assert.match(guide, /Epic 0017 is implemented and benchmarked/);
  assert.match(guide, /project-documents\/issues\/86/);
  assert.match(guide, /project-documents\/issues\/73/);
  assert.match(guide, /crates\/graph-core\/tests\/epic_0017_acceptance\.rs/);
  assert.match(
    guide,
    /feature-0017-learned-working-set\/artifacts\/0017-reproducibility-report\.md/,
  );
  assert.match(architecture, /Epic 0017 acceptance suite and reproducibility report are complete/);

  // Reject the former future-tense wording so a later edit cannot silently
  // regress the completed epic to an in-progress product claim.
  for (const staleStatus of [
    "Epic 0017 is in progress",
    "remain open in issue #274",
    "once merged",
    "The remaining Epic 0017 acceptance report",
  ]) {
    assert.ok(
      !guide.includes(staleStatus) && !architecture.includes(staleStatus),
      `documentation must not retain stale status: ${staleStatus}`,
    );
  }
});
