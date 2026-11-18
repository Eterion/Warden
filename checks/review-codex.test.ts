import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, it } from 'node:test';
import { codexExe, withoutAppAliases } from './review-codex.ts';

describe('review-codex', () => {
  it('hides app execution aliases from Codex, and nothing else', () => {
    const env = withoutAppAliases({
      Path: [
        '%SystemRoot%\\System32',
        '%USERPROFILE%\\AppData\\Local\\Microsoft\\WindowsApps\\Microsoft.PowerShell_8wekyb3d8bbwe',
        '%USERPROFILE%\\AppData\\Local\\Microsoft\\WindowsApps',
        '%ProgramFiles%\\tools',
        '%ProgramFiles%\\MyWindowsAppsTool',
      ].join(';'),
      OTHER: 'x',
    });
    assert.equal(env.Path, '%SystemRoot%\\System32;%ProgramFiles%\\tools;%ProgramFiles%\\MyWindowsAppsTool');
    assert.equal(env.OTHER, 'x');
  });

  it('finds codex.exe on PATH, the first one, and nothing when only a codex.cmd is there', () => {
    const root = mkdtempSync(join(tmpdir(), 'review-codex-'));
    try {
      const none = join(root, 'none');
      const cmd = join(root, 'cmd');
      const exe = join(root, 'exe');
      const later = join(root, 'later');
      for (const dir of [none, cmd, exe, later]) mkdirSync(dir);
      writeFileSync(join(cmd, 'codex.cmd'), '');
      writeFileSync(join(exe, 'codex.exe'), '');
      writeFileSync(join(later, 'codex.exe'), '');
      assert.equal(codexExe({ Path: [none, cmd, exe, later].join(';') }), join(exe, 'codex.exe'));
      assert.equal(codexExe({ PATH: [none, cmd].join(';') }), undefined);
      assert.equal(codexExe({}), undefined);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});
