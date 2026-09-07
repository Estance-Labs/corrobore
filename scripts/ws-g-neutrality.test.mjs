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
