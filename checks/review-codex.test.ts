import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { withoutAppAliases } from './review-codex.ts';

describe('review-codex', () => {
  it('hides app execution aliases from Codex, and nothing else', () => {
    const env = withoutAppAliases({
      Path: '%SystemRoot%\\System32;%USERPROFILE%\\AppData\\Local\\Microsoft\\WindowsApps;%ProgramFiles%\\tools',
      OTHER: 'x',
    });
    assert.equal(env.Path, '%SystemRoot%\\System32;%ProgramFiles%\\tools');
    assert.equal(env.OTHER, 'x');
  });
});
