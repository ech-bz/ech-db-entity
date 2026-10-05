import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import * as schema from '../src/schema.gen.ts';

const here = path.dirname(fileURLToPath(import.meta.url));
const casesPath = path.join(here, '../../../tests/fixtures/entity/bcs_cases.json');
interface BcsCase {
  name: string;
  schema: string;
  hex: string;
  json: unknown;
}

const cases: BcsCase[] = JSON.parse(readFileSync(casesPath, 'utf8'));

function normalize(value: unknown): unknown {
  if (typeof value === 'bigint') {
    return Number(value);
  }
  if (Array.isArray(value)) {
    return value.map(normalize);
  }
  if (value !== null && typeof value === 'object') {
    const out: Record<string, unknown> = {};
    for (const [key, inner] of Object.entries(value)) {
      out[key] = normalize(inner);
    }
    return out;
  }
  return value;
}

const decoders = schema as unknown as Record<string, { parse(bytes: Uint8Array): unknown }>;

for (const item of cases) {
  test(`decodes ${item.name}`, () => {
    const decoder = decoders[item.schema];
    assert.ok(decoder, `missing schema ${item.schema}`);
    const bytes = Uint8Array.from(Buffer.from(item.hex, 'hex'));
    const decoded = normalize(decoder.parse(bytes));
    assert.deepEqual(decoded, item.json);
  });
}
