/** Resolve the most recently completed Pages publication from verified job steps. */
export async function findPublishedDocsRevision({ repository, requestJson }) {
  if (
    typeof repository !== "string" ||
    !/^[-\w]+\/[-\w]+$/.test(repository) ||
    typeof requestJson !== "function"
  )
    throw Error("Invalid publication history request");
  const publications = [];
  async function pages(route, key) {
    const entries = [];
    for (let page = 1; page <= 20; page++) {
      const response = await requestJson(`${route}&per_page=100&page=${page}`),
        rows = response?.[key];
      if (!Array.isArray(rows)) throw Error("Malformed publication history");
      entries.push(...rows);
      if (rows.length < 100) return entries;
    }
    throw Error(
      "Publication history exceeds bounded pagination; cannot establish baseline",
    );
  }
  const runs = await pages(
    `/repos/${repository}/actions/workflows/docs.yml/runs?status=success`,
    "workflow_runs",
  );
  for (const run of runs) {
    if (
      !Number.isSafeInteger(run.id) ||
      run.id < 1 ||
      typeof run.head_sha !== "string" ||
      !/^[a-f0-9]{40}$/.test(run.head_sha)
    )
      throw Error("Invalid documentation run identity");
    const jobs = await pages(
      `/repos/${repository}/actions/runs/${run.id}/jobs?filter=all`,
      "jobs",
    );
    for (const job of jobs) {
      if (job.name !== "deploy") continue;
      if (!Array.isArray(job.steps))
        throw Error("Missing deployment step history");
      for (const step of job.steps) {
        if (
          step.name !== "Deploy to GitHub Pages" ||
          step.conclusion !== "success"
        )
          continue;
        const completed = Date.parse(step.completed_at);
        if (!Number.isFinite(completed))
          throw Error("Invalid publication completion time");
        publications.push({
          revision: run.head_sha,
          runId: run.id,
          publishedAt: step.completed_at,
          completed,
        });
      }
    }
  }
  publications.sort((a, b) => b.completed - a.completed || b.runId - a.runId);
  if (!publications.length)
    throw Error("No verified published documentation baseline");
  const { completed, ...result } = publications[0];
  return result;
}
