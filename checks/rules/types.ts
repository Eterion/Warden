import type { SourceFile } from '../lib/repo.ts';

export interface Violation {
  rule: string;
  path: string;
  line: number;
  message: string;
}

export interface Rule {
  name: string;
  /** What the rule protects, shown when it fails. */
  why: string;
  /** Whether the rule looks at this repo-relative path at all. */
  applies(path: string): boolean;
  check(file: SourceFile): Violation[];
}

/** Every match of `pattern` in `text`, with its 1-based line number and the whole line. */
export function matches(
  text: string,
  pattern: RegExp,
): { line: number; match: string; lineText: string; index: number }[] {
  const flags = pattern.flags.includes('g') ? pattern.flags : pattern.flags + 'g';
  const re = new RegExp(pattern.source, flags);
  const lines = text.split('\n');
  const out: { line: number; match: string; lineText: string; index: number }[] = [];
  lines.forEach((lineText, i) => {
    for (const m of lineText.matchAll(re)) out.push({ line: i + 1, match: m[0], lineText, index: m.index });
  });
  return out;
}

/** The rule scripts' own tests plant violations on purpose. */
export function isRuleFixture(path: string): boolean {
  return path.startsWith('checks/rules/') && path.endsWith('.test.ts');
}

export function isGenerated(path: string): boolean {
  return path === 'package-lock.json' || path === 'Cargo.lock';
}
