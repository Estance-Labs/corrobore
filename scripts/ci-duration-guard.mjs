import process from "node:process";

const DEFAULT_MAX_RUNS = 30;
const DEFAULT_BRANCH = "main";
const DEFAULT_WORKFLOW = "rust-ci.yml";

function parseArgs(argv) {
  const args = {
    branch: DEFAULT_BRANCH,
    maxRuns: DEFAULT_MAX_RUNS,
    workflow: DEFAULT_WORKFLOW,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const token = argv[i];
    const next = argv[i + 1];

    if (token === "--branch" && next) {
      args.branch = next;
      i += 1;
      continue;
    }

    if (token === "--max-runs" && next) {
      const parsed = Number.parseInt(next, 10);
      if (!Number.isFinite(parsed) || parsed <= 0) {
        throw new Error(`invalid --max-runs value: ${next}`);
      }
      args.maxRuns = parsed;
      i += 1;
      continue;
    }

    if (token === "--workflow" && next) {
      args.workflow = next;
      i += 1;
      continue;
    }

    throw new Error(`unknown argument: ${token}`);
  }

  return args;
}

export function percentile(values, p) {
  if (!Array.isArray(values) || values.length === 0) {
    throw new Error("percentile requires a non-empty array");
  }

  if (p <= 0 || p > 100) {
    throw new Error("percentile p must be in (0, 100]");
  }

  const sorted = [...values].sort((a, b) => a - b);
  const rank = Math.max(0, Math.ceil((p / 100) * sorted.length) - 1);
  return sorted[rank];
}

export function summarizeDurations(durationsMinutes) {
  if (!Array.isArray(durationsMinutes) || durationsMinutes.length === 0) {
    throw new Error("summarizeDurations requires at least one duration");
  }

  const durations = durationsMinutes.filter((value) => Number.isFinite(value) && value > 0);

  if (durations.length === 0) {
    throw new Error("no valid positive durations available");
  }

  const total = durations.reduce((sum, value) => sum + value, 0);

  return {
    count: durations.length,
    min: Math.min(...durations),
    max: Math.max(...durations),
    mean: total / durations.length,
    p50: percentile(durations, 50),
    p95: percentile(durations, 95),
  };
}

function extractDurationMinutes(run) {
  const start = Date.parse(run.created_at ?? "");
  const end = Date.parse(run.updated_at ?? "");

  if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start) {
    return null;
  }

  return (end - start) / 60_000;
}

function round2(value) {
  return Math.round(value * 100) / 100;
}

async function fetchWorkflowRuns({ owner, repo, workflow, branch, token }) {
  const url = new URL(`https://api.github.com/repos/${owner}/${repo}/actions/workflows/${workflow}/runs`);
  url.searchParams.set("branch", branch);
  url.searchParams.set("status", "completed");
  url.searchParams.set("per_page", "100");

  const response = await fetch(url, {
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${token}`,
      "User-Agent": "corrobore-ci-duration-guard",
      "X-GitHub-Api-Version": "2022-11-28",
    },
  });

  if (!response.ok) {
    const details = await response.text();
    throw new Error(`GitHub API request failed (${response.status}): ${details}`);
  }

  const payload = await response.json();
  return payload.workflow_runs ?? [];
}

async function main() {
  const { branch, maxRuns, workflow } = parseArgs(process.argv.slice(2));
  const repository = process.env.GITHUB_REPOSITORY;
  const token = process.env.GITHUB_TOKEN;

  if (!repository || !repository.includes("/")) {
    throw new Error("GITHUB_REPOSITORY must be set as owner/repo");
  }

  if (!token) {
    throw new Error("GITHUB_TOKEN must be set");
  }

  const [owner, repo] = repository.split("/");
  const threshold = process.env.P95_MAX_MINUTES
    ? Number.parseFloat(process.env.P95_MAX_MINUTES)
    : null;

  if (threshold !== null && (!Number.isFinite(threshold) || threshold <= 0)) {
    throw new Error("P95_MAX_MINUTES must be a positive number when set");
  }

  const runs = await fetchWorkflowRuns({ owner, repo, workflow, branch, token });
  const successfulRuns = runs.filter((run) => run.conclusion === "success").slice(0, maxRuns);
  const durations = successfulRuns
    .map((run) => extractDurationMinutes(run))
    .filter((value) => value !== null);

  const stats = summarizeDurations(durations);

  console.log(`Workflow: ${workflow}`);
  console.log(`Branch: ${branch}`);
  console.log(`Samples: ${stats.count}`);
  console.log(`Duration min/mean/p50/p95/max (minutes): ${round2(stats.min)} / ${round2(stats.mean)} / ${round2(stats.p50)} / ${round2(stats.p95)} / ${round2(stats.max)}`);

  if (threshold !== null && stats.p95 > threshold) {
    console.error(`p95 duration ${round2(stats.p95)} exceeds threshold ${threshold}`);
    process.exitCode = 2;
    return;
  }

  if (threshold !== null) {
    console.log(`p95 threshold check passed (<= ${threshold} minutes)`);
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}