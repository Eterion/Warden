import { describe, expect, it } from 'vitest';
import type { Session } from './api';
import {
  BIN_MS,
  nowLabel,
  runningFor,
  sessionName,
  sessionTree,
  sortSessions,
  sparkPaths,
  sparkPoints,
  subagentsLabel,
  tally,
} from './sessions';

const T = 1_794_053_400_000; // a multiple of 5 minutes

function s(id: string, state: Session['state'], extra: Partial<Session> = {}): Session {
  return {
    parentId: null,
    id,
    provider: 'claude',
    accountId: 'a',
    state,
    doing: '',
    project: null,
    name: null,
    startedAt: T,
    lastEventAt: T,
    endedAt: null,
    subagents: [],
    signedInAs: null,
    source: 'hook',
    ...extra,
  };
}

describe('the sessions tree', () => {
  const shape = (list: Session[]) => sessionTree(list).map((r) => `${'  '.repeat(r.depth)}${r.session.id}`);

  it('nests each session under the one that started it, children of children too', () => {
    expect(
      shape([
        s('m1', 'working', { startedAt: T - 3 }),
        s('c1', 'working', { parentId: 'm1', startedAt: T - 2 }),
        s('g1', 'idle', { parentId: 'c1' }),
        s('m2', 'idle', { startedAt: T - 1 }),
      ]),
    ).toEqual(['m1', '  c1', '    g1', 'm2']);
  });

  it('puts waiting rows first among siblings, and a parent does not move for a waiting child', () => {
    expect(
      shape([
        s('m1', 'working', { startedAt: T - 9 }),
        s('m2', 'idle', { startedAt: T - 8 }),
        s('c1', 'working', { parentId: 'm2', startedAt: T - 7 }),
        s('c2', 'waiting', { parentId: 'm2', startedAt: T - 6 }),
        s('m3', 'waiting', { startedAt: T - 5 }),
      ]),
    ).toEqual(['m3', 'm1', 'm2', '  c2', '  c1']);
  });

  it('keeps a session whose parent is unknown or gone at the top level', () => {
    const rows = sessionTree([s('m1', 'idle', { startedAt: T - 1 }), s('orphan', 'working', { parentId: 'gone' })]);
    expect(rows.map((r) => [r.session.id, r.depth, r.parent, r.faded])).toEqual([
      ['orphan', 0, null, false],
      ['m1', 0, null, false],
    ]);
    // Without `from`, a parent left out of the list is gone too.
    const all = [s('m1', 'idle'), s('c1', 'working', { parentId: 'm1', accountId: 'b' })];
    expect(shape(all.filter((x) => x.accountId === 'b'))).toEqual(['c1']);
  });

  it("puts a filter's sessions under their ancestors from `from`, faded, and leaves the rest out", () => {
    const all = [
      s('root', 'done', { accountId: 'a', startedAt: T - 9, endedAt: T }),
      s('mid', 'waiting', { accountId: 'a', parentId: 'root', startedAt: T - 8 }),
      s('b1', 'working', { accountId: 'b', parentId: 'mid', startedAt: T - 7 }),
      s('b2', 'idle', { accountId: 'b', parentId: 'root', startedAt: T - 6 }),
      s('other', 'working', { accountId: 'a', startedAt: T - 5 }),
      s('sibling', 'working', { accountId: 'a', parentId: 'mid', startedAt: T - 4 }),
      s('b3', 'done', { accountId: 'b', parentId: 'gone', endedAt: T }),
    ];
    const rows = sessionTree(
      all.filter((x) => x.accountId === 'b'),
      all,
    );
    expect(rows.map((r) => [`${'  '.repeat(r.depth)}${r.session.id}`, r.faded, r.parent?.id ?? null])).toEqual([
      ['root', true, null],
      ['  mid', true, 'root'],
      ['    b1', false, 'mid'],
      ['  b2', false, 'root'],
      ['b3', false, null],
    ]);
    // The connectors follow the tree with its faded rows in it.
    expect(rows.map((r) => [r.last, r.rails])).toEqual([
      [false, []],
      [false, []],
      [true, [true]],
      [true, []],
      [true, []],
    ]);
    // A session that matches is never faded, even as another's ancestor; a loop of ancestors (cut at the top level, as
    // without `from`) ends where it closes.
    expect(sessionTree([all[1]!, all[2]!], all).map((r) => [r.session.id, r.faded])).toEqual([
      ['root', true],
      ['mid', false],
      ['b1', false],
    ]);
    const loop = [
      s('x', 'idle', { parentId: 'y' }),
      s('y', 'idle', { parentId: 'x' }),
      s('c', 'idle', { parentId: 'x' }),
    ];
    expect(sessionTree([loop[2]!], loop).map((r) => [r.session.id, r.depth, r.faded])).toEqual([
      ['c', 0, false],
      ['x', 0, true],
      ['y', 0, true],
    ]);
  });

  it('cuts a loop of parents, which a real link never makes, at the top level', () => {
    expect(shape([s('x', 'idle', { parentId: 'y' }), s('y', 'idle', { parentId: 'x' })])).toEqual(['x', 'y']);
    expect(shape([s('self', 'idle', { parentId: 'self' })])).toEqual(['self']);
  });

  it('marks the last sibling and the lines that run past a row', () => {
    const rows = sessionTree([
      s('m', 'working', { startedAt: T - 9 }),
      s('a', 'working', { parentId: 'm', startedAt: T - 8 }),
      s('a1', 'working', { parentId: 'a', startedAt: T - 7 }),
      s('b', 'working', { parentId: 'm', startedAt: T - 6 }),
      s('b1', 'working', { parentId: 'b', startedAt: T - 5 }),
    ]);
    expect(rows.map((r) => [r.session.id, r.last, r.rails])).toEqual([
      ['m', true, []],
      ['a', false, []],
      ['a1', true, [true]],
      ['b', true, []],
      ['b1', true, [false]],
    ]);
  });

  it('names a session by its own name, else the end of its id', () => {
    const rows = sessionTree([s('m', 'idle', { name: 'Overview design' }), s('c0ffee1234', 'idle', { parentId: 'm' })]);
    expect(sessionName(rows[1]!.parent!)).toBe('Overview design');
    expect(sessionName(rows[1]!.session)).toBe('Session ffee1234');
  });

  it('tells apart two unnamed sessions whose time-ordered ids start alike', () => {
    const a = s('019a7f3c-0000-7000-8000-00000000a1b2', 'idle');
    const b = s('019a7f3c-0000-7000-8000-00000000c3d4', 'idle');
    expect(sessionName(a)).toBe('Session 0000a1b2');
    expect(sessionName(b)).toBe('Session 0000c3d4');
  });
});

describe('sessions', () => {
  it('sorts waiting first and done last', () => {
    const sorted = sortSessions([
      s('d', 'done'),
      s('i', 'idle'),
      s('l', 'idle', { source: 'list' }),
      s('w', 'working'),
      s('a', 'waiting'),
    ]);
    expect(sorted.map((x) => x.id)).toEqual(['a', 'w', 'i', 'l', 'd']);
  });

  it('counts working, waiting and idle, not listed or done', () => {
    expect(tally([s('1', 'working'), s('2', 'idle', { source: 'list' }), s('3', 'done'), s('4', 'idle')])).toEqual({
      working: 1,
      waiting: 0,
      idle: 1,
    });
  });

  it('says how long a session has run', () => {
    expect(runningFor(s('1', 'working'), T + 38 * 60_000)).toBe('38m');
    expect(runningFor(s('1', 'working'), T + 62 * 60_000)).toBe('1h 02m');
    expect(runningFor(s('1', 'done', { endedAt: T + 5 * 60_000 }), T + 99 * 60_000)).toBe('5m');
  });

  it('fills the two hours, carrying counts through bins without a change', () => {
    const now = T + 2 * 60_000;
    const points = sparkPoints(
      [
        { at: T - 30 * BIN_MS, working: 5, last: 1, waited: false },
        { at: T - 2 * BIN_MS, working: 3, last: 2, waited: true },
      ],
      now,
    );
    expect(points).toHaveLength(24);
    expect(points[0]).toEqual({ working: 1, waited: false });
    expect(points[21]).toEqual({ working: 3, waited: true });
    expect(points[22]).toEqual({ working: 2, waited: false });
    expect(points[23]).toEqual({ working: 2, waited: false });
  });

  it('draws a step line on one scale with a dot per wait', () => {
    const points = Array.from({ length: 24 }, (_, k) => ({ working: k < 12 ? 0 : 1, waited: k === 12 }));
    const p = sparkPaths(points, 2, 'Main');
    expect(p.line.startsWith('M2 25 H6')).toBe(true);
    expect(p.line).toContain('V14.5');
    expect(p.area.endsWith('V25 H2 Z')).toBe(true);
    expect(p.dots).toEqual([{ x: 52, y: 14.5 }]);
    expect(p.label).toBe('Main, last 2 hours: up to 1 session working, waited for you once');
    const flat = points.map(() => ({ working: 0, waited: false }));
    expect(sparkPaths(flat, 1, 'X').label).toBe('X, last 2 hours: nothing working');
  });
});

describe('session labels', () => {
  it('sorts by state, then the longest running first within a state, and a listed session after idle ones', () => {
    const list = [
      s('late', 'working', { startedAt: T + 10 }),
      s('listed', 'working', { source: 'list' }),
      s('idle', 'idle'),
      s('early', 'working', { startedAt: T - 10 }),
    ];
    expect(sortSessions(list).map((x) => x.id)).toEqual(['early', 'late', 'idle', 'listed']);
    expect(list[0]!.id).toBe('late');
  });

  it('says what is going on now, waiting first, or nothing when no session counts', () => {
    expect(nowLabel({ working: 2, waiting: 1, idle: 0 })).toBe('Now: 1 waiting for you, 2 working');
    expect(nowLabel({ working: 0, waiting: 0, idle: 3 })).toBe('Now: 3 idle');
    expect(nowLabel({ working: 0, waiting: 0, idle: 0 })).toBeNull();
  });

  it('names one subagent or several', () => {
    expect(subagentsLabel(s('a', 'working', { subagents: [{ type: 'Explore' }] }))).toBe('1 subagent: Explore');
    expect(subagentsLabel(s('a', 'working', { subagents: [{ type: 'Explore' }, { type: 'Plan' }] }))).toBe(
      '2 subagents: Explore, Plan',
    );
  });

  it('counts a finished session up to when it ended, and never below zero', () => {
    const MIN = 60_000;
    expect(runningFor(s('a', 'done', { endedAt: T + 75 * MIN }), T + 500 * MIN)).toBe('1h 15m');
    expect(runningFor(s('a', 'working', { startedAt: T + MIN }), T)).toBe('0m');
  });
});
