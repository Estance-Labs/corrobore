# OpenCTI Intelligence Harvester Agent Skill

The canonical OpenCTI harvester now ships as a discovered Agent Skill at
`plugins/corrobore/skills/opencti-intel-harvester/SKILL.md`.

[Read the canonical SKILL.md](https://github.com/Estance-Labs/corrobore/blob/main/plugins/corrobore/skills/opencti-intel-harvester/SKILL.md)

Install the complete Corrobore Agent Plugin so the harvester can also load the
general Corrobore operating rules through package-local links.

See [Agent Plugin](../../agent-skill.md) for download and installation details.

For extracted assertions, the workflow is: submit, read the failing constraint,
re-extract that field, resubmit. Keep raw versions in Shadow or Hypothesis and
use explicit source-reviewed promotion; do not write extraction directly to
canonical records. See the [candidate ingestion contract](../../user-guide/ingestion.md#candidate-ingestion-ws-c)
and the [package-local recipe](https://github.com/Estance-Labs/corrobore/blob/main/plugins/corrobore/skills/corrobore/references/candidate-ingestion.md).
