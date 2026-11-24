import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { type RouteAttempt, type RouteRound, selectRouteAttempt } from './budgets.ts';

const BUDGET = { silent: 25, print: 25 };

function rounds(count: number, silent: number, print: number, controls: RouteRound['controls']): RouteRound[] {
  return Array.from({ length: count }, () => ({ silent, print, controls }));
}

function attempt(attempts: number, samples: RouteRound[]): RouteAttempt {
  const median = (values: number[]) => values.sort((a, b) => a - b)[Math.floor(values.length / 2)]!;
  return {
    rounds: samples,
    times: {
      attempts,
      silent: median(samples.map((round) => round.silent)),
      print: median(samples.map((round) => round.print)),
      control: median(samples.flatMap((round) => round.controls)),
      seconds: 12,
    },
  };
}

const passing = attempt(2, [...rounds(30, 8, 9, [8, 8, 8]), ...rounds(11, 8, 9, [10, 10, 10])]);

function assertBreachKept(samples: RouteRound[]): void {
  const breached = attempt(1, samples);
  assert.deepEqual(selectRouteAttempt([breached], passing, BUDGET), { times: breached.times, excusedAttempts: [] });
}

describe('route budget retries', () => {
  it('keeps a breach with normal controls', () => {
    assertBreachKept(rounds(41, 28, 9, [10, 10, 10]));
  });

  it('keeps a breach if only one neighboring control slowed in a failing mode', () => {
    // The silent launch is bracketed by slow controls, but the printing launch has a fast control after it.
    assertBreachKept(rounds(41, 33, 34, [30, 30, 10]));
  });

  it('keeps a route regression of a few milliseconds when the route and controls share an equal delay', () => {
    // Both launches take 20 ms longer in the burst, leaving the route itself at 28 ms.
    assertBreachKept(rounds(41, 48, 29, [30, 30, 30]));
  });

  it('keeps a breach with only 20 shared rounds', () => {
    assertBreachKept([
      ...rounds(20, 33, 9, [30, 30, 30]),
      ...rounds(1, 33, 9, [10, 10, 10]),
      ...rounds(20, 8, 9, [10, 10, 10]),
    ]);
  });

  it('keeps a breach when the median of the remaining rounds is over budget', () => {
    assertBreachKept([...rounds(21, 33, 9, [30, 30, 30]), ...rounds(20, 28, 9, [10, 10, 10])]);
  });

  it('excuses each attempt affected by a real burst and returns its evidence', () => {
    const first = attempt(1, [...rounds(21, 33, 34, [30, 30, 30]), ...rounds(20, 8, 9, [10, 10, 10])]);
    const second = attempt(2, rounds(41, 24, 35, [31, 31, 31]));
    const last = attempt(3, passing.rounds);
    assert.deepEqual(selectRouteAttempt([first, second], last, BUDGET), {
      times: last.times,
      excusedAttempts: [
        { times: first.times, baselineMs: 10, sharedRounds: { silent: 21, print: 21 } },
        { times: second.times, baselineMs: 10, sharedRounds: { print: 41 } },
      ],
    });
  });

  it('never excuses a breach without a later passing measurement', () => {
    const first = attempt(1, rounds(41, 33, 9, [30, 30, 30]));
    const last = attempt(2, rounds(41, 35, 9, [31, 31, 31]));
    assert.deepEqual(selectRouteAttempt([first], last, BUDGET), { times: last.times, excusedAttempts: [] });
  });
});
