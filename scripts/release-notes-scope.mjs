import { execFile } from "node:child_process";
import { promisify } from "node:util";
import path from "node:path";
const execute = promisify(execFile);
/** Detect unpublished versioned-note changes, including retries after a failed deployment. */
export async function findReleaseNoteChanges({
  repositoryRoot = process.cwd(),
  baseline,
  candidate,
}) {
  for (const revision of [baseline, candidate])
    if (typeof revision !== "string" || !/^[a-f0-9]{40}$/.test(revision))
      throw Error("Immutable published and candidate revisions are required");
  const root = path.resolve(repositoryRoot);
  const git = async (args) =>
    (
      await execute("git", ["-C", root, ...args], {
        encoding: "utf8",
        timeout: 10000,
        maxBuffer: 4 * 1024 * 1024,
      })
    ).stdout;
  for (const revision of [baseline, candidate])
    if ((await git(["cat-file", "-t", revision])).trim() !== "commit")
      throw Error("Release-note baseline and candidate must be commits");
  await git(["merge-base", "--is-ancestor", baseline, candidate]);
  const changed = await git([
    "diff",
    "--no-ext-diff",
    "--no-renames",
    "--name-only",
    "-z",
    "--diff-filter=AM",
    baseline,
    candidate,
    "--",
    "docs/release-notes/",
  ]);
  const files = changed
    .split("\0")
    .filter((file) => /^docs\/release-notes\/v[^/]+\.md$/.test(file))
    .sort();
  return {
    schemaVersion: "corrobore-release-note-scope-v1",
    baseline,
    candidate,
    required: files.length > 0,
    files,
  };
}
