import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { problem } from './commits.ts';

describe('commit messages', () => {
  it('accepts Conventional Commits', () => {
    for (const ok of [
      'feat(ui): settings search',
      'fix: keep the last reading',
      'docs: AGENTS.md imports',
      'refactor(core)!: rename the store',
      'build(checks): Prettier',
    ]) {
      assert.equal(problem(ok), null, ok);
    }
  });

  it('catches everything else', () => {
    for (const bad of [
      'Server: long poll for changes',
      'Fix the code reviewer findings',
      'feature(ui): x',
      'fix(UI): x',
      'fix:no space',
      'fix(): x',
      'fix: ',
    ]) {
      assert.notEqual(problem(bad), null, bad);
    }
    assert.match(problem('feature(ui): x') ?? '', /unknown type/);
  });
});
