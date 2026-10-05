import { BoardEntityRecord } from './schema.gen.ts';
import type { BoardEntity } from './schema.gen.ts';

export interface BoardRendered {
  version: number;
  title: string;
  slug: string;
  lines: string[];
  locked: boolean;
}

export function hex(bytes: readonly number[]): string {
  return Array.from(bytes)
    .map((byte) => byte.toString(16).padStart(2, '0'))
    .join('');
}

export function decodeBoardRecord(bytes: Uint8Array) {
  const record = BoardEntityRecord.parse(bytes);
  if (record.type_name !== 'BoardEntity') {
    throw new Error(`expected BoardEntity record, found ${record.type_name}`);
  }
  return record;
}

export function decodeBoardState(bytes: Uint8Array): BoardEntity {
  return decodeBoardRecord(bytes).state;
}

export function renderBoard(state: BoardEntity): BoardRendered {
  switch (state.$kind) {
    case 'V1':
      return renderV1(state.V1);
    case 'V2':
      return renderV2(state.V2);
    case 'V3':
      return renderV3(state.V3);
    default: {
      const unknown = state as { $kind: string };
      throw new Error(`unknown BoardEntity version: ${unknown.$kind}`);
    }
  }
}

function renderV1(state: {
  slug: string;
  title: string;
  admin: readonly number[];
  threads: readonly number[];
}): BoardRendered {
  return {
    version: 1,
    title: state.title,
    slug: state.slug,
    lines: [`admin: ${hex(state.admin)}`, `threads: ${hex(state.threads)}`],
    locked: false,
  };
}

function renderV2(state: {
  slug: string;
  title: string;
  admin: readonly number[];
  threads: readonly number[];
  locked: boolean;
}): BoardRendered {
  return {
    version: 2,
    title: state.title,
    slug: state.slug,
    lines: [`admin: ${hex(state.admin)}`, `threads: ${hex(state.threads)}`],
    locked: state.locked,
  };
}

function renderV3(state: {
  slug: string;
  title: string;
  threads: readonly number[];
  locked: boolean;
  moderators: readonly number[];
}): BoardRendered {
  return {
    version: 3,
    title: state.title,
    slug: state.slug,
    lines: [`threads: ${hex(state.threads)}`, `moderators: ${hex(state.moderators)}`],
    locked: state.locked,
  };
}
