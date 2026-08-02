// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(fileURLToPath(import.meta.url));
const timestamp = '2026-01-15T10:00:00.000Z';
const stixId = (type, suffix) => `${type}--00000000-0000-4000-8000-${String(suffix).padStart(12, '0')}`;
const common = (type, suffix, fields) => ({
  type,
  spec_version: '2.1',
  id: stixId(type, suffix),
  created: timestamp,
  modified: timestamp,
  ...fields,
});

const intrusion = common('intrusion-set', 1, {
  name: 'Synthetic Orchard',
  aliases: ['Orchard Group'],
  description: 'A fictional intrusion set used only for Corrobore acceptance testing.',
});
const malware = [
  common('malware', 101, { name: 'Copper Finch', is_family: true, malware_types: ['remote-access-trojan'] }),
  common('malware', 102, { name: 'Silver Fern', is_family: true, malware_types: ['loader'] }),
  common('malware', 103, {
    name: 'Amber Reed',
    is_family: true,
    malware_types: ['backdoor'],
    extensions: {
      'extension-definition--00000000-0000-4000-8000-000000009999': {
        synthetic_score: 7,
        note: 'Unknown extension retained verbatim by the supported boundary.',
      },
    },
  }),
];
const techniques = [
  ['Command and Scripting Interpreter', 'T1059'],
  ['Ingress Tool Transfer', 'T1105'],
  ['Process Injection', 'T1055'],
  ['Exfiltration Over Web Service', 'T1567'],
].map(([name, externalId], index) => common('attack-pattern', 201 + index, {
  name,
  external_references: [{ source_name: 'synthetic-attack', external_id: externalId }],
}));
const sectors = [
  common('identity', 301, { name: 'Synthetic Energy Cooperative', identity_class: 'organization', sectors: ['energy'] }),
  common('identity', 302, { name: 'Synthetic Transport Cooperative', identity_class: 'organization', sectors: ['transportation'] }),
];
const locations = [
  common('location', 401, { name: 'Synthetic North Region', region: 'northern-europe' }),
  common('location', 402, { name: 'Synthetic West Region', region: 'western-europe' }),
];
const files = [
  {
    type: 'file',
    id: stixId('file', 501),
    name: 'orchard-loader.bin',
    hashes: { 'SHA-256': '1111111111111111111111111111111111111111111111111111111111111111' },
  },
  {
    type: 'file',
    id: stixId('file', 502),
    name: 'orchard-config.dat',
    hashes: { 'SHA-256': '2222222222222222222222222222222222222222222222222222222222222222' },
  },
];
const domains = [
  { type: 'domain-name', id: stixId('domain-name', 601), value: 'update.synthetic.invalid' },
  { type: 'domain-name', id: stixId('domain-name', 602), value: 'cdn.synthetic.invalid' },
];

const nodes = [intrusion, ...malware, ...techniques, ...sectors, ...locations, ...files, ...domains];
const relationshipSpecs = [];
for (const target of malware) relationshipSpecs.push(['uses', intrusion, target]);
for (const target of techniques) relationshipSpecs.push(['uses', intrusion, target]);
for (let index = 0; index < malware.length; index += 1) {
  for (let offset = 0; offset < 3; offset += 1) {
    relationshipSpecs.push(['uses', malware[index], techniques[(index + offset) % techniques.length]]);
  }
}
for (const target of [...sectors, ...locations]) relationshipSpecs.push(['targets', intrusion, target]);
for (const source of malware) {
  for (const target of domains) relationshipSpecs.push(['communicates-with', source, target]);
}
for (let index = 0; index < malware.length; index += 1) {
  relationshipSpecs.push(['drops', malware[index], files[index % files.length]]);
}

const relationships = relationshipSpecs.map(([relationshipType, source, target], index) => common(
  'relationship',
  1001 + index,
  { relationship_type: relationshipType, source_ref: source.id, target_ref: target.id },
));
if (relationships.length !== 29) throw new Error(`expected 29 base relationships, got ${relationships.length}`);

const report = common('report', 701, {
  name: 'Synthetic Orchard campaign report',
  report_types: ['threat-report'],
  published: timestamp,
  object_refs: [...nodes.map(({ id }) => id), ...relationships.map(({ id }) => id)],
});
const reportRelationship = common('relationship', 1030, {
  relationship_type: 'related-to',
  source_ref: report.id,
  target_ref: intrusion.id,
});
relationships.push(reportRelationship);

const evidencePayloads = [
  ['evidence--report-page-2', 'The report names Synthetic Orchard and three fictional malware families.', { type: 'page', page: 2 }],
  ['evidence--report-paragraph-4-2', 'Synthetic Orchard uses T1059, T1105, T1055 and T1567-like techniques.', { type: 'paragraph', page: 4, paragraph: 2 }],
  ['evidence--report-table-6-1-2-3', 'update.synthetic.invalid', { type: 'table_cell', page: 6, table: 1, row: 2, column: 3 }],
];
const sourceText = evidencePayloads.map(([, payload]) => payload).join('\n');
const contentSha256 = createHash('sha256').update(sourceText).digest('hex');
const evidence = {
  schema_version: '1.0',
  records: evidencePayloads.map(([id, payload, locator]) => ({
    id,
    source_id: 'document--synthetic-orchard-report-v1',
    content_sha256: contentSha256,
    payload,
    locator,
    extractor_id: 'fixture-generator/v1',
    model_version: 'none-deterministic-structured-input',
    language: 'en',
  })),
  annotations: {},
};

const allObjects = [...relationships.slice().reverse(), report, ...nodes];
for (let index = 0; index < allObjects.length; index += 1) {
  evidence.annotations[allObjects[index].id] = {
    evidence_refs: [evidence.records[index % evidence.records.length].id],
    confidence: 92,
    status: 'candidate',
  };
}
evidence.annotations[malware[1].id].confidence = 40;
evidence.annotations[domains[1].id].evidence_refs = [];
evidence.annotations[domains[1].id].confidence = 90;

const input = {
  schema_version: '1.0',
  description: 'Already-extracted synthetic CTI candidates; no PDF, OCR, or LLM extraction is performed by Corrobore.',
  bundle: { type: 'bundle', id: stixId('bundle', 1), objects: allObjects },
  evidence,
  workspace_id: 'workspace--report-to-stix-acceptance',
  session_id: 'session--report-to-stix-acceptance',
  budget_ref: 'budget--report-to-stix-acceptance',
};
const expected = {
  schema_version: '1.0',
  expected_import: { requested: 47, nodes: 17, relationships: 30, evidence_records: 3 },
  validation_issue_codes: ['CTI_CONFIDENCE_TOO_LOW', 'CTI_EVIDENCE_REQUIRED'],
  corrections: {
    low_confidence_id: malware[1].id,
    missing_evidence_id: domains[1].id,
    evidence_id: evidence.records[2].id,
    final_confidence: 0.95,
    final_status: 'exportable',
  },
  golden: {
    object_count: 47,
    intrusion_set_id: intrusion.id,
    malware_ids: malware.map(({ id }) => id),
    technique_external_ids: techniques.map(({ external_references }) => external_references[0].external_id),
    report_id: report.id,
    report_object_ref_count: report.object_refs.length,
    unknown_extension_object_id: malware[2].id,
  },
  negative_cases: [
    { id: 'dangling-reference', expected_code: 'UNRESOLVED_STIX_REFERENCE' },
    { id: 'contradictory-candidate', expected_code: 'CONFLICTING_STIX_ID' },
    { id: 'unknown-extension', expected_outcome: 'preserved' },
    { id: 'invalid-list', expected_code: 'UNSUPPORTED_PARAMETER_TYPE' },
    { id: 'provider-not-ready', expected_code: 'DOMAIN_PROVIDER_NOT_READY' },
    { id: 'license-missing', expected_code: 'LICENSE_MODULE_MISSING' },
  ],
};

const stringify = (value) => `${JSON.stringify(value, null, 2)}\n`;
writeFileSync(join(root, 'input.json'), stringify(input));
writeFileSync(join(root, 'expected.json'), stringify(expected));

const checksums = ['input.json', 'expected.json'].map((name) => {
  const bytes = Buffer.from(name === 'input.json' ? stringify(input) : stringify(expected));
  return `${createHash('sha256').update(bytes).digest('hex')}  ${name}`;
}).join('\n');
writeFileSync(join(root, 'checksums.sha256'), `${checksums}\n`);
