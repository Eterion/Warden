import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { reviewKinds } from './review.ts';

describe('review by production risk', () => {
  it('skips tests, docs and checks, even mixed together', () => {
    assert.deepEqual(
      reviewKinds([
        'README.md',
        'docs/api.md',
        'checks/api-doc.ts',
        'ui/src/api.test.ts',
        'crates/warden-core/tests/routing.rs',
      ]),
      [],
    );
  });
  it('keeps rule documents in code review at any depth, while ordinary Markdown needs none', () => {
    assert.deepEqual(reviewKinds(['AGENTS.md']), ['code']);
    assert.deepEqual(reviewKinds(['README.md']), []);
    for (const path of [
      'CLAUDE.md',
      'ui/AGENTS.md',
      'checks/CLAUDE.md',
      'crates/warden-core/tests/AGENTS.md',
      'ui/src/CLAUDE.md',
    ])
      assert.ok(reviewKinds([path]).includes('code'), path);
  });
  it('requests UX only for production UI', () => {
    assert.deepEqual(reviewKinds(['ui/src/settings/AppSection.vue', 'checks/api-doc.ts']), ['ux']);
    assert.deepEqual(reviewKinds(['ui/public/icon.svg']), ['ux']);
  });

  it('keeps dev security and configuration in code review', () => {
    for (const path of [
      'ui/dev/origin.ts',
      'ui/dev/token.ts',
      'ui/package.json',
      'ui/tsconfig.json',
      'ui/tsconfig.app.json',
      'ui/eslint.config.js',
    ])
      assert.deepEqual(reviewKinds([path]), ['code'], path);
    for (const path of ['ui/index.html', 'ui/vite.config.ts'])
      assert.deepEqual(reviewKinds([path]), ['code', 'ux'], path);
    assert.deepEqual(reviewKinds(['ui/dev/origin.test.ts']), []);
    assert.deepEqual(reviewKinds(['ui/dev/token.ts', 'ui/src/App.vue']), ['code', 'ux']);
  });

  it('keeps enforcement files in code review ahead of test/doc/check exclusions', () => {
    for (const path of [
      'checks/rules/run.ts',
      'checks/rules/rules.test.ts',
      'checks/budgets.ts',
      'checks/budgets.json',
      'checks/lib/review.ts',
      'checks/lib/selection.ts',
      'checks/lib/pages.ts',
      'checks/run.ts',
      'checks/a11y.ts',
      'checks/contrast.ts',
      'checks/contrast.test.ts',
      'crates/warden-server/tests/rules.rs',
      'crates/warden-rules/src/lib.rs',
      'crates/warden-rules/tests/rules.rs',
      'crates/warden-rules/README.md',
      '.agents/reviewers/code-reviewer.md',
      '.claude/settings.json',
      '.claude/agents/ux-reviewer.md',
    ])
      assert.deepEqual(reviewKinds([path]), ['code'], path);
    assert.deepEqual(reviewKinds(['ui/src/styles/contrast-pairs.json']), ['code', 'ux']);
    assert.deepEqual(reviewKinds(['checks/lib/review.test.ts', 'checks/run.test.ts', 'checks/lib/colour.test.ts']), []);
  });
  it('requests code for Rust sources and unknown build inputs', () => {
    assert.deepEqual(reviewKinds(['crates/warden-core/src/lib.rs']), ['code']);
    assert.deepEqual(reviewKinds(['Cargo.toml']), ['code']);
  });
  it('requests both reviews when runtime Rust and UI change together', () => {
    assert.deepEqual(reviewKinds(['crates/warden-core/src/lib.rs', 'ui/src/App.vue']), ['code', 'ux']);
  });
});
