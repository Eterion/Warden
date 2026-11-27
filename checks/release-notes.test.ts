import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, it } from 'node:test';
import { ROOT } from './lib/repo.ts';
import { releaseNotes } from './release-notes.ts';

const CHANGELOG = [
  '# Changelog',
  '',
  '## 0.3.0',
  '',
  '### New',
  '',
  '- A feature.',
  '',
  '### Fixes and improvements',
  '',
  '- A fix.',
  '',
  '## 0.2.2',
  '',
  '### New',
  '',
  '### Fixes and improvements',
  '',
  '## 0.2.1',
  '',
  '## 0.2.0',
  '',
  '- Older.',
  '',
].join('\r\n');

describe('release notes', () => {
  it("takes the version's section up to the next version, without its heading", () => {
    assert.equal(releaseNotes(CHANGELOG, '0.3.0'), '### New\n\n- A feature.\n\n### Fixes and improvements\n\n- A fix.');
    assert.equal(releaseNotes(CHANGELOG, '0.2.0'), '- Older.');
  });

  it('refuses a missing section', () => {
    assert.throws(() => releaseNotes(CHANGELOG, '0.4.0'), /no "## 0\.4\.0" section/);
    assert.throws(() => releaseNotes(CHANGELOG, '0.3'), /no "## 0\.3" section/);
  });

  it('refuses an empty section', () => {
    assert.throws(() => releaseNotes(CHANGELOG, '0.2.1'), /"## 0\.2\.1" section is empty/);
  });

  it('refuses a section with only headings', () => {
    assert.throws(() => releaseNotes(CHANGELOG, '0.2.2'), /"## 0\.2\.2" section has only headings/);
  });

  it('reads the newest section of the real changelog', () => {
    const changelog = readFileSync(join(ROOT, 'CHANGELOG.md'), 'utf8');
    const newest = /^## (\S+)/m.exec(changelog)?.[1] ?? '';
    assert.match(releaseNotes(changelog, newest), /^### /);
  });
});
