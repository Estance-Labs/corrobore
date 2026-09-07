import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
const read=name=>readFile(new URL(`../${name}`,import.meta.url),'utf8');
test('neutral collection primitives contain structure without pack-specific assessment vocabulary',async()=>{
 const source=await read('crates/graph-core/src/narrative_campaign.rs');
 assert.doesNotMatch(source,/\bfimi\b|misleadingness|unsupported_inference|emotional_arousal|communicative_intent|reader_interpretation|generation_fingerprint/i);
 const manifest=await read('crates/graph-core/Cargo.toml');
 assert.doesNotMatch(manifest,/fimi/i);
});
test('coordination signals carry provenance vocabulary without pack-specific assessment vocabulary',async()=>{
 const source=await read('crates/graph-core/src/campaign_signals.rs');
 assert.doesNotMatch(source,/\bfimi\b|misleadingness|unsupported_inference|emotional_arousal|communicative_intent|reader_interpretation/i);
 // A coordination signal is provenance, never authorship: the refusal must stay.
 assert.match(source,/CoordinationSignalsOnly/);
 assert.match(source,/fn affects_independence/);
});
test('the FIMI exporter keeps assessments and verdicts in separate fields',async()=>{
 const source=await read('crates/export-fimi/src/lib.rs');
 // An assessment is never a verdict, and coordination evidence is never an author.
 assert.match(source,/not_a_factual_determination/);
 assert.match(source,/ATTRIBUTION_NOT_ASSERTED/);
 // The exporter carries the pack's recorded band; it must not compute one.
 assert.doesNotMatch(source,/fn\s+\w*band\w*\s*\(/);
});
