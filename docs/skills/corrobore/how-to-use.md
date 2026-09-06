# Corrobore Agent Skill

The canonical skill now ships inside the portable Agent Plugin package at
`plugins/corrobore/skills/corrobore/SKILL.md`.

[Read the canonical SKILL.md](https://github.com/Estance-Labs/corrobore/blob/main/plugins/corrobore/skills/corrobore/SKILL.md)

Use the complete package rather than copying this documentation page. Keeping
the manifest, both skills, and their references together preserves portable
discovery and package-local links.

See [Agent Plugin](../../agent-skill.md) for download and installation details.

For extracted assertions, the workflow is: submit, read the failing constraint,
re-extract that field, resubmit. Keep raw versions in Shadow or Hypothesis and
use explicit source-reviewed promotion; do not write extraction directly to
canonical records. See the [candidate ingestion contract](../../user-guide/ingestion.md#candidate-ingestion-ws-c)
and the [package-local recipe](https://github.com/Estance-Labs/corrobore/blob/main/plugins/corrobore/skills/corrobore/references/candidate-ingestion.md).
