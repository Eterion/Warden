import { isTest } from './selection.ts';

export type ReviewKind = 'code' | 'ux';

/** Enforcement files need code review even when they are tests, docs or check scripts. */
function enforcesRules(path: string): boolean {
  return (
    /(?:^|\/)(?:AGENTS|CLAUDE)\.md$/.test(path) ||
    path.startsWith('checks/rules/') ||
    /^checks\/(?:budgets\.(?:ts|json)|lib\/(?:review|selection|pages)\.ts|run\.ts|a11y\.ts|contrast[^/]*)$/.test(
      path,
    ) ||
    path === 'ui/src/styles/contrast-pairs.json' ||
    path === 'crates/warden-server/tests/rules.rs' ||
    path.startsWith('crates/warden-rules/') ||
    path.startsWith('.agents/') ||
    path.startsWith('.claude/')
  );
}

/** Only rendered source/assets are UX-only; configuration, dev security and enforcement also need code review. */
export function reviewKinds(files: string[]): ReviewKind[] {
  const production = files.filter(
    (path) =>
      enforcesRules(path) ||
      (!isTest(path) && !path.startsWith('checks/') && !/\.(?:md|txt)$/.test(path) && !path.startsWith('docs/')),
  );
  const rendered = (path: string) => path.startsWith('ui/src/') || path.startsWith('ui/public/');
  const ui = production.some(
    (path) => (rendered(path) && !isTest(path)) || path === 'ui/index.html' || path === 'ui/vite.config.ts',
  );
  const code = production.some((path) => enforcesRules(path) || !rendered(path));
  return [...(code ? ['code' as const] : []), ...(ui ? ['ux' as const] : [])];
}
