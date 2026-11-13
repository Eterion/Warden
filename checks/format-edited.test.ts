import assert from 'node:assert/strict';
import { join } from 'node:path';
import { describe, it } from 'node:test';
import { editedPath } from './format-edited.ts';
import { ROOT } from './lib/repo.ts';

const input = (file_path: unknown) => JSON.stringify({ tool_name: 'Edit', tool_input: { file_path } });

describe('format-edited hook', () => {
  it('finds the edited file inside the repo', () => {
    assert.equal(editedPath(input(join(ROOT, 'ui', 'src', 'App.vue'))), 'ui/src/App.vue');
    assert.equal(editedPath(input('checks/run.ts')), 'checks/run.ts');
  });

  it('ignores files outside the repo and input it cannot read', () => {
    assert.equal(editedPath(input(join(ROOT, '..', 'elsewhere.ts'))), null);
    assert.equal(editedPath(input(undefined)), null);
    assert.equal(editedPath('not json'), null);
  });
});
