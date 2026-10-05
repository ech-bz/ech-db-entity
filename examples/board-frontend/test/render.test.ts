import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { decodeBoardRecord, decodeBoardState, renderBoard } from '../src/frontend.ts';

const here = path.dirname(fileURLToPath(import.meta.url));
const casesPath = path.join(here, '../../../tests/fixtures/entity/bcs_cases.json');
interface RenderCase {
  name: string;
  hex: string;
}

const cases: RenderCase[] = JSON.parse(readFileSync(casesPath, 'utf8'));

function bytesFor(name: string): Uint8Array {
  const item = cases.find((candidate) => candidate.name === name);
  assert.ok(item, `missing case ${name}`);
  return Uint8Array.from(Buffer.from(item.hex, 'hex'));
}

test('renders the stored V1 version without migrating', () => {
  const rendered = renderBoard(decodeBoardState(bytesFor('record_board_v1')));
  assert.equal(rendered.version, 1);
  assert.equal(rendered.title, 'Rust Board');
  assert.equal(rendered.lines.length, 2);
  assert.equal(rendered.locked, false);
});

test('renders the stored V2 version without migrating', () => {
  const rendered = renderBoard(decodeBoardState(bytesFor('record_board_v2')));
  assert.equal(rendered.version, 2);
  assert.equal(rendered.locked, true);
});

test('renders the stored V3 version', () => {
  const rendered = renderBoard(decodeBoardState(bytesFor('record_board_v3')));
  assert.equal(rendered.version, 3);
  assert.equal(rendered.lines.length, 2);
});

test('decoding does not mutate the input', () => {
  const bytes = bytesFor('record_board_v3');
  const before = Array.from(bytes);
  decodeBoardRecord(bytes);
  assert.deepEqual(Array.from(bytes), before);
});

test('unknown state version is an explicit read error', () => {
  const bytes = bytesFor('record_board_v3');
  bytes[45] = 9;
  assert.throws(() => decodeBoardState(bytes));
});

test('foreign entity record is an explicit read error', () => {
  assert.throws(() => decodeBoardState(bytesFor('record_thread_v1')));
});
