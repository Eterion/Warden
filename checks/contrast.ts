// WCAG AA contrast check on the design tokens, in light and dark.
//
// ui/src/styles/contrast-pairs.json lists how tokens are used together: a foreground on a stack of backgrounds
// (bottom first; translucent layers are composited), and the minimum ratio: 4.5 for text, 3 for large text,
// icons, bars and control outlines. A pair may hold only for one theme with "themes": ["light"].

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { contrast, over, parseColour, themeValues, type Rgba } from './lib/colour.ts';
import { ROOT } from './lib/repo.ts';

type Theme = 'light' | 'dark';

interface Pair {
  fg: string;
  bg: string[];
  min: number;
  use: string;
  themes?: Theme[];
}

export function readColourTokens(css: string): Map<string, { light: string; dark: string }> {
  const tokens = new Map<string, { light: string; dark: string }>();
  for (const m of css.matchAll(/--color-([\w-]+):\s*([^;]+);/g)) {
    const [, name, value] = m;
    if (!name || !value || value.trim() === 'initial' || value.includes('currentColor')) continue;
    tokens.set(name, themeValues(value));
  }
  return tokens;
}

export function checkPairs(
  tokens: Map<string, { light: string; dark: string }>,
  pairs: Pair[],
): { line: string; ok: boolean }[] {
  const colour = (name: string, theme: Theme): Rgba => {
    const t = tokens.get(name);
    if (!t) throw new Error(`contrast-pairs.json names an unknown token "${name}"`);
    return parseColour(t[theme]);
  };
  const results: { line: string; ok: boolean }[] = [];
  for (const pair of pairs) {
    for (const theme of pair.themes ?? (['light', 'dark'] as Theme[])) {
      // Everything sits on an opaque base: the bottom layer must be opaque.
      const bg = pair.bg.map((n) => colour(n, theme)).reduce((acc, layer) => over(layer, acc));
      if (bg.a < 1) throw new Error(`background stack ${pair.bg.join(' > ')} is not opaque in ${theme}`);
      const fg = over(colour(pair.fg, theme), bg);
      const ratio = contrast(fg, bg);
      const ok = ratio >= pair.min;
      results.push({
        ok,
        line: `${ok ? 'ok  ' : 'FAIL'} ${theme.padEnd(5)} ${ratio.toFixed(2).padStart(5)} (min ${pair.min})  ${pair.fg} on ${pair.bg.join(' > ')}: ${pair.use}`,
      });
    }
  }
  return results;
}

function main(): void {
  const css = readFileSync(join(ROOT, 'ui/src/styles/tokens.css'), 'utf8');
  const pairs = JSON.parse(readFileSync(join(ROOT, 'ui/src/styles/contrast-pairs.json'), 'utf8')) as Pair[];
  const results = checkPairs(readColourTokens(css), pairs);
  const failed = results.filter((r) => !r.ok);
  const verbose = process.argv.includes('--verbose');
  for (const r of verbose ? results : failed) console.log(r.line);
  console.log(`contrast: ${results.length - failed.length} of ${results.length} token pairs meet WCAG AA`);
  if (failed.length) process.exitCode = 1;
}

if (import.meta.main) main();
