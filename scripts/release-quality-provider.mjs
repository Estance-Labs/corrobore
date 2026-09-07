import {
  readFile,
  writeFile,
  mkdir,
  mkdtemp,
  rm,
  realpath,
  lstat,
} from "node:fs/promises";
import path from "node:path";
import { tmpdir } from "node:os";
import { createHash } from "node:crypto";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
const execute = promisify(execFile);
const invalid = (detail) => {
  throw Error(`Release quality provider rejected: ${detail}`);
};
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
async function configuredPin() {
  return JSON.parse(
    await readFile(
      new URL("./release-quality-provider.json", import.meta.url),
      "utf8",
    ),
  );
}
/** Verify pinned executable bytes even when Git index flags conceal a local edit. */
export async function verifyReleaseProvider(providerRoot, pin = undefined) {
  pin ??= await configuredPin();
  if (
    pin?.schemaVersion !== "corrobore-release-quality-provider-v1" ||
    typeof pin.repository !== "string" ||
    !/^[-\w]+\/[-\w]+$/.test(pin.repository) ||
    !/^([a-f0-9]{40})$/.test(pin.revision ?? "") ||
    !pin.files ||
    typeof pin.files !== "object" ||
    Array.isArray(pin.files) ||
    !Object.hasOwn(pin.files, pin.entrypoint)
  )
    invalid("pin manifest");
  const root = await realpath(providerRoot);
  const git = async (...args) =>
    (
      await execute("git", ["-C", root, ...args], {
        encoding: "utf8",
        timeout: 10000,
        maxBuffer: 2 * 1024 * 1024,
      })
    ).stdout.trim();
  if ((await realpath(await git("rev-parse", "--show-toplevel"))) !== root)
    invalid("provider must be a checkout root");
  if ((await git("rev-parse", "HEAD")) !== pin.revision)
    invalid("revision differs from pin");
  const origin = await git("remote", "get-url", "origin");
  if (
    ![
      `https://github.com/${pin.repository}.git`,
      `https://github.com/${pin.repository}`,
      `git@github.com:${pin.repository}.git`,
    ].includes(origin)
  )
    invalid("repository differs from pin");
  if (await git("status", "--porcelain=v1", "--untracked-files=all"))
    invalid("provider checkout must be clean");
  const files = new Map();
  for (const [name, expected] of Object.entries(pin.files)) {
    if (
      !name.startsWith("scripts/") ||
      name
        .split("/")
        .some(
          (part) =>
            !part ||
            part === "." ||
            part === ".." ||
            !/^[a-zA-Z0-9_.-]+$/.test(part),
        ) ||
      typeof expected !== "string" ||
      !/^[a-f0-9]{64}$/.test(expected)
    )
      invalid("unsafe file path or digest");
    const file = path.join(root, name),
      info = await lstat(file);
    if (
      !info.isFile() ||
      info.isSymbolicLink() ||
      (await realpath(file)) !== file ||
      info.size > 4 * 1024 * 1024
    )
      invalid(`runtime file: ${name}`);
    const bytes = await readFile(file);
    if (digest(bytes) !== expected) invalid(`runtime file digest: ${name}`);
    files.set(name, bytes);
  }
  return {
    repository: pin.repository,
    revision: pin.revision,
    entrypoint: pin.entrypoint,
    files,
  };
}
/** Copy verified bytes before execution so later checkout edits cannot replace them. */
export async function runPinnedQualityGate(options, pin = undefined) {
  const verified = await verifyReleaseProvider(options.providerRoot, pin);
  const snapshot = await mkdtemp(
    path.join(tmpdir(), "corrobore-quality-provider-"),
  );
  try {
    for (const [name, bytes] of verified.files) {
      const target = path.join(snapshot, name);
      await mkdir(path.dirname(target), { recursive: true });
      await writeFile(target, bytes, { flag: "wx" });
    }
    const args = [
      path.join(snapshot, verified.entrypoint),
      "--bundle",
      path.resolve(options.bundle),
      "--candidate",
      options.candidate,
      "--baseline",
      options.baseline,
      "--release-ref",
      options.releaseRef,
      "--release-path",
      options.releasePath,
      "--output-dir",
      path.resolve(options.outputDir),
    ];
    try {
      const result = await execute(process.execPath, args, {
        encoding: "utf8",
        timeout: 120000,
        maxBuffer: 4 * 1024 * 1024,
      });
      return { exitCode: 0, ...result, providerRevision: verified.revision };
    } catch (error) {
      if (!Number.isInteger(error.code)) throw error;
      return {
        exitCode: error.code,
        stdout: error.stdout ?? "",
        stderr: error.stderr ?? "",
        providerRevision: verified.revision,
      };
    }
  } finally {
    await rm(snapshot, { recursive: true, force: true });
  }
}
