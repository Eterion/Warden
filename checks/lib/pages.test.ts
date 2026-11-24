import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { PAGES } from '../a11y.ts';
import { affectedPages } from './pages.ts';
import { readFileSync } from 'node:fs';
import { ROOT } from './repo.ts';

describe('render only affected states, with conservative shared fallbacks', () => {
  it('derives common dependencies from new App and modal-frame imports', () => {
    for (const [entry, dependency] of [
      ['ui/src/App.vue', './settings/AppSection.vue'],
      ['ui/src/settings/SettingsModal.vue', './AppSection.vue'],
    ] as const) {
      const readSource = (path: string) => {
        const text = readFileSync(`${ROOT}/${path}`, 'utf8');
        return path === entry
          ? text.replace('</script>', `import NewFramePart from '${dependency}';\n</script>`)
          : text;
      };
      // This dependency is already known to App's settings page. A shell import widens it to every state.
      assert.deepEqual(affectedPages(PAGES, ['ui/src/settings/AppSection.vue'], readSource), PAGES, entry);
    }
  });
  it('keeps every state of each settings section, in the browser and desktop', () => {
    assert.deepEqual(
      affectedPages(PAGES, ['ui/src/settings/AppSection.vue']).map((p) => p.name),
      ['settings-app', 'settings-search', 'desktop-settings-app'],
    );
    assert.deepEqual(
      affectedPages(PAGES, ['ui/src/settings/AppearanceSection.vue']).map((p) => p.name),
      ['settings-appearance', 'desktop-settings-appearance'],
    );
  });

  it('does not expand a screen change just because docs or tests changed with it', () => {
    assert.deepEqual(
      affectedPages(PAGES, ['AGENTS.md', 'ui/src/api.test.ts', 'ui/src/settings/AppSection.vue']),
      affectedPages(PAGES, ['ui/src/settings/AppSection.vue']),
    );
  });

  it('follows transitive modal imports instead of maintaining component lists', () => {
    const selected = affectedPages(PAGES, ['ui/src/settings/AccountEditModal.vue']);
    assert.ok(selected.some((p) => p.name === 'settings-rename-account'));
    assert.ok(selected.every((p) => p.path.includes('/accounts')));
  });

  it('includes underlying Accounts states for the add-account screen', () => {
    const selected = affectedPages(PAGES, ['ui/src/settings/AccountsSection.vue']);
    assert.ok(selected.some((p) => p.name === 'desktop-add-existing-folder'));
    assert.ok(selected.some((p) => p.name === 'settings-rename-account'));
    assert.ok(!selected.some((p) => ['settings-app', 'settings-appearance'].includes(p.name)));
  });

  it('visits everything for shared, deleted, unknown or tool inputs', () => {
    for (const path of [
      'ui/src/App.vue',
      'ui/src/styles/tokens.css',
      'ui/src/components/WButton.vue',
      'ui/src/api.ts',
      'ui/src/store.ts',
      'ui/src/pages/OverviewPage.vue',
      'ui/src/settings/Deleted.vue',
      'checks/fixtures/ui-state.ts',
      'checks/a11y.ts',
      'package-lock.json',
    ])
      assert.deepEqual(affectedPages(PAGES, [path]), PAGES, path);
    assert.deepEqual(
      affectedPages(
        [...PAGES, { name: 'new', path: '/#/unknown', fixture: 'normal' }],
        ['ui/src/settings/AppSection.vue'],
      ).length,
      PAGES.length + 1,
    );
  });

  it('needs no rendered states for UI test-only changes', () => {
    assert.deepEqual(affectedPages(PAGES, ['ui/src/settings/AppSection.test.ts', 'ui/src/test/state.ts']), []);
  });
});
