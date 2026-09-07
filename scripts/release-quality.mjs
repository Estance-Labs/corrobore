import { parseArgs } from "node:util";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { runPinnedQualityGate } from "./release-quality-provider.mjs";
const fields = {
  provider: "providerRoot",
  bundle: "bundle",
  candidate: "candidate",
  baseline: "baseline",
  "release-ref": "releaseRef",
  "release-path": "releasePath",
  "output-dir": "outputDir",
};
/** Bind the exact publication and reference independently of candidate evidence. */
export function parseReleaseQualityArgs(argv) {
  const { values, tokens } = parseArgs({
    args: argv,
    options: Object.fromEntries(
      Object.keys(fields).map((key) => [key, { type: "string" }]),
    ),
    strict: true,
    allowPositionals: false,
    tokens: true,
  });
  const seen = new Set();
  for (const token of tokens) {
    if (seen.has(token.name))
      throw Error(`Duplicate release quality option --${token.name}`);
    seen.add(token.name);
  }
  for (const key of Object.keys(fields))
    if (!values[key]?.trim())
      throw Error(`Required release quality option --${key}`);
  for (const key of ["candidate", "baseline"])
    if (!/^[a-f0-9]{40}$/.test(values[key]))
      throw Error(`Immutable ${key} revision is required`);
  if (values.candidate === values.baseline)
    throw Error("Candidate and approved baseline must be distinct");
  if (
    ![
      "engine-release",
      "docker-release",
      "agent-plugin-release",
      "release-notes-publication",
    ].includes(values["release-path"])
  )
    throw Error("Unsupported core publication family");
  if (
    /[\s\u0000-\u001f\u007f-\u009f]/u.test(values["release-ref"]) ||
    Buffer.byteLength(values["release-ref"]) > 256
  )
    throw Error("Invalid release reference");
  return Object.fromEntries(
    Object.entries(fields).map(([key, name]) => [name, values[key]]),
  );
}
/** Delegate to the pinned implementation without duplicating its acceptance rules. */
export async function runCoreReleaseQuality(
  argv,
  { runGate = runPinnedQualityGate } = {},
) {
  return runGate(parseReleaseQualityArgs(argv));
}
if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  try {
    const result = await runCoreReleaseQuality(process.argv.slice(2));
    process.stdout.write(result.stdout);
    process.stderr.write(result.stderr);
    process.exitCode = result.exitCode;
  } catch (error) {
    process.stderr.write(`Release quality blocked: ${error.message}\n`);
    process.exitCode = 1;
  }
}
