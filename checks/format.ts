// Prettier on TypeScript, JavaScript, Vue, CSS, JSON, YAML and Markdown (.prettierrc.json; .gitignore and
// .prettierignore are skipped). Rust is formatted by rustfmt instead.
//
//   node checks/format.ts                      check every file
//   node checks/format.ts --base <ref>         check only the files changed since <ref> (with uncommitted ones)
//   node checks/format.ts --write [files...]   format every file, or only these

import { readFile, writeFile } from 'node:fs/promises';
import { join, relative, resolve } from 'node:path';
import * as prettier from 'prettier';
import { ROOT, changedFiles, listFiles } from './lib/repo.ts';

export const PRETTIER_FILES = /\.(ts|mts|cts|js|mjs|cjs|vue|css|json|ya?ml|md)$/i;

const IGNORE = [join(ROOT, '.gitignore'), join(ROOT, '.prettierignore')];

/**
 * Formats one file with Prettier, or only checks it. Returns whether it was already formatted; `null` when Prettier
 * does not handle the file (another type, ignored or missing).
 */
export async function formatFile(path: string, write: boolean): Promise<boolean | null> {
  if (!PRETTIER_FILES.test(path)) return null;
  const abs = join(ROOT, path);
  const info = await prettier.getFileInfo(abs, { ignorePath: IGNORE, resolveConfig: true });
  if (info.ignored || !info.inferredParser) return null;
  let text: string;
  try {
    text = await readFile(abs, 'utf8');
  } catch {
    return null; // deleted
  }
  const options = { ...(await prettier.resolveConfig(abs)), filepath: abs };
  if (await prettier.check(text, options)) return true;
  if (write) await writeFile(abs, await prettier.format(text, options));
  return false;
}

function arg(name: string): string | undefined {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

async function main(): Promise<void> {
  const write = process.argv.includes('--write');
  const base = arg('base');
  const named = process.argv.slice(2).filter((a, i, all) => !a.startsWith('--') && all[i - 1] !== '--base');
  const paths = named.length
    ? named.map((p) => relative(ROOT, resolve(p)).replaceAll('\\', '/'))
    : base
      ? changedFiles(base, false).files
      : listFiles();
  const unformatted: string[] = [];
  let checked = 0;
  for (const path of paths) {
    const ok = await formatFile(path, write);
    if (ok === null) continue;
    checked++;
    if (!ok) unformatted.push(path);
  }
  if (write) {
    console.log(`prettier: formatted ${unformatted.length} of ${checked} files`);
    return;
  }
  for (const p of unformatted) console.error(`not formatted: ${p}`);
  console.log(
    unformatted.length
      ? `prettier: ${unformatted.length} of ${checked} files not formatted; run npm run format`
      : `prettier: ${checked} files formatted`,
  );
  if (unformatted.length) process.exitCode = 1;
}

if (import.meta.main) await main();
