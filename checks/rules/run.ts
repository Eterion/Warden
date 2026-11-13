// Runs every rule script over the repository. Exit code 1 when any rule is broken.
//
//   node checks/rules/run.ts            all files
//   node checks/rules/run.ts a.ts b.vue only these files

import { listFiles, readFiles } from '../lib/repo.ts';
import { cliFallback } from './cli-fallback.ts';
import { personalPaths } from './personal-paths.ts';
import { tokensOnly } from './tokens-only.ts';
import type { Rule, Violation } from './types.ts';
import type { SourceFile } from '../lib/repo.ts';

export const RULES: Rule[] = [personalPaths, tokensOnly, cliFallback];

export function checkFiles(files: SourceFile[], rules: Rule[] = RULES): Violation[] {
  return rules.flatMap((rule) => files.filter((f) => rule.applies(f.path)).flatMap((f) => rule.check(f)));
}

function main(): void {
  const args = process.argv.slice(2).map((p) => p.replaceAll('\\', '/'));
  const files = readFiles(args.length ? args : listFiles());
  const violations = checkFiles(files);
  if (violations.length === 0) {
    console.log(`rules: ${RULES.map((r) => r.name).join(', ')} pass on ${files.length} files`);
    return;
  }
  for (const v of violations) console.error(`${v.path}:${v.line}: [${v.rule}] ${v.message}`);
  for (const rule of RULES.filter((r) => violations.some((v) => v.rule === r.name))) {
    console.error(`\n[${rule.name}] ${rule.why}`);
  }
  console.error(`\nrules: ${violations.length} violation(s)`);
  process.exitCode = 1;
}

if (import.meta.main) main();
