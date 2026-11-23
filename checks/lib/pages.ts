import { existsSync, readFileSync } from 'node:fs';
import { dirname, posix } from 'node:path';
import ts from 'typescript';
import { parse } from 'vue/compiler-sfc';
import { ROOT } from './repo.ts';
import { isTest } from './selection.ts';

export interface PageState {
  name: string;
  path: string;
  fixture: string;
}

/** Resolve the same relative modules used by Vite. Unresolved local imports force a full visit. */
function resolveImport(from: string, name: string): string {
  const base = posix.normalize(posix.join(dirname(from).replaceAll('\\', '/'), name));
  const found = [base, `${base}.ts`, `${base}.vue`, `${base}.js`, `${base}/index.ts`].find((path) =>
    existsSync(`${ROOT}/${path}`),
  );
  if (!found) throw new Error(`unresolved import ${name} in ${from}`);
  return found;
}

/** Read both SFC scripts and TS imports with their parser, including re-exports and literal dynamic imports. */
function imports(path: string, readSource: (path: string) => string): Map<string, string> {
  const text = readSource(path);
  let scripts = text;
  if (path.endsWith('.vue')) {
    const { descriptor, errors } = parse(text);
    if (errors.length) throw new Error(`cannot parse ${path}`);
    // An unscoped style can affect every screen, even when its component is not visible.
    if (descriptor.styles.some((style) => !style.scoped || style.src)) throw new Error(`global style in ${path}`);
    scripts = [descriptor.script?.content, descriptor.scriptSetup?.content].filter(Boolean).join('\n');
    if (descriptor.script?.src || descriptor.scriptSetup?.src) throw new Error(`external script in ${path}`);
  }
  const source = ts.createSourceFile(path, scripts, ts.ScriptTarget.Latest, true);
  const result = new Map<string, string>();
  const add = (name: string, alias: string) => {
    if (name.startsWith('.')) result.set(alias, resolveImport(path, name));
    else if (name.startsWith('@/') || name.startsWith('/')) throw new Error(`non-relative local import in ${path}`);
  };
  const visit = (node: ts.Node) => {
    if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) {
      if (node.moduleSpecifier && ts.isStringLiteral(node.moduleSpecifier)) {
        add(
          node.moduleSpecifier.text,
          ts.isImportDeclaration(node)
            ? (node.importClause?.name?.text ?? node.moduleSpecifier.text)
            : node.moduleSpecifier.text,
        );
      }
    }
    if (ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword) {
      const name = node.arguments[0];
      if (!name || !ts.isStringLiteral(name)) throw new Error(`non-literal dynamic import in ${path}`);
      add(name.text, name.text);
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return result;
}

/**
 * The route selects an imported Section in SettingsModal; all states of that route share its import closure.
 * App and modal-frame imports are common roots; routed screen imports stay specific to their page states.
 * A new route that does not follow this convention runs all pages.
 */
export function affectedPages<T extends PageState>(
  pages: T[],
  changed: string[],
  readSource = (path: string) => readFileSync(`${ROOT}/${path}`, 'utf8'),
): T[] {
  const relevant = changed.filter((path) => !isTest(path) && !/\.(?:md|txt)$/.test(path) && !path.startsWith('docs/'));
  if (!relevant.length) return [];
  try {
    const app = imports('ui/src/App.vue', readSource);
    const settings = imports(app.get('SettingsModal') ?? '', readSource);
    const cache = new Map<string, Set<string>>();
    function closure(path: string, seen = new Set<string>()): Set<string> {
      if (seen.has(path)) return seen;
      seen.add(path);
      if (/\.(?:vue|ts|js)$/.test(path))
        for (const dependency of imports(path, readSource).values()) closure(dependency, seen);
      return seen;
    }
    const routedAliases = new Set<string>();
    const pageRoots = pages.map((page) => {
      const roots: string[] = [];
      if (page.fixture.startsWith('setup')) {
        const setup = app.get('SetupModal');
        if (!setup) throw new Error('unknown setup entry');
        roots.push(setup);
      }
      const route = page.path.split('#')[1]?.replace(/^\//, '').split('/');
      if (route?.[0] === 'settings') {
        const name = `${route[1]}Section`.toLowerCase();
        const section = [...settings].find(([alias]) => alias.toLowerCase() === name);
        if (!section) throw new Error('unknown settings route');
        routedAliases.add(section[0]);
        roots.push(section[1]);
        if (route[2] === 'add') {
          const add = settings.get('AddAccountModal');
          if (!add) throw new Error('unknown add-account entry');
          routedAliases.add('AddAccountModal');
          roots.push(add);
        } else if (route[2]) throw new Error('unknown nested route');
      } else if (route?.[0] === 'memory') {
        // The Memory page is one of App's own imports, so its closure is common to every state.
      } else if (route?.[0]) throw new Error('unknown route');
      return roots;
    });
    // No list of sidebar, strip or frame components: new shell imports become common automatically.
    const commonRoots = [
      ...[...app].filter(([alias]) => alias !== 'SettingsModal' && alias !== 'SetupModal').map(([, path]) => path),
      ...[...settings].filter(([alias]) => !routedAliases.has(alias)).map(([, path]) => path),
    ];
    const dependencies = pageRoots.map((roots) => {
      return new Set(
        [...commonRoots, ...roots].flatMap((root) => {
          if (!cache.has(root)) cache.set(root, closure(root));
          return [...cache.get(root)!];
        }),
      );
    });
    // Anything outside a known screen closure is shared or unknown. Shared components always visit all states.
    if (
      relevant.some(
        (path) =>
          path.endsWith('.css') ||
          path.startsWith('ui/src/components/') ||
          !dependencies.some((deps) => deps.has(path)),
      )
    )
      return pages;
    return pages.filter((_, index) => relevant.some((path) => dependencies[index]?.has(path)));
  } catch {
    return pages;
  }
}
