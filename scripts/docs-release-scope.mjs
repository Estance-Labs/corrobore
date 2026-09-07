import { appendFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { findPublishedDocsRevision } from "./docs-publication-history.mjs";
import { findReleaseNoteChanges } from "./release-notes-scope.mjs";
/** Keep publication history and stage-quality baseline identities separate. */
export async function inspectDocsRelease({
  repositoryRoot = process.cwd(),
  repository,
  candidate,
  requestJson,
}) {
  const publication = await findPublishedDocsRevision({
    repository,
    requestJson,
  });
  const scope = await findReleaseNoteChanges({
    repositoryRoot,
    baseline: publication.revision,
    candidate,
  });
  return { ...scope, publication };
}
async function main() {
  if (process.argv.length !== 2)
    throw Error(
      "Documentation scope accepts its trusted workflow environment, not positional overrides",
    );
  const token = process.env.GITHUB_TOKEN;
  if (!token) throw Error("Read access to publication history is required");
  const result = await inspectDocsRelease({
    repository: process.env.GITHUB_REPOSITORY,
    candidate: process.env.GITHUB_SHA,
    requestJson: async (route) => {
      const response = await fetch(`https://api.github.com${route}`, {
        headers: {
          Accept: "application/vnd.github+json",
          Authorization: `Bearer ${token}`,
          "X-GitHub-Api-Version": "2022-11-28",
        },
        redirect: "error",
        signal: AbortSignal.timeout(30000),
      });
      if (!response.ok)
        throw Error(`Publication history request failed (${response.status})`);
      return response.json();
    },
  });
  if (process.env.GITHUB_OUTPUT)
    await appendFile(
      process.env.GITHUB_OUTPUT,
      `required=${result.required}\nbaseline=${result.baseline}\n`,
    );
  process.stdout.write(JSON.stringify(result) + "\n");
}
if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
)
  main().catch((error) => {
    process.stderr.write(
      `Documentation publication blocked: ${error.message}\n`,
    );
    process.exitCode = 1;
  });
