// Rule: the UI uses only the design tokens. Raw colours and sizes appear only in the tokens file, and line height
// is set once, on the root, in the tokens file.

import { matches, type Rule, type Violation } from './types.ts';

export const TOKENS_FILE = 'ui/src/styles/tokens.css';

const SCANNED = /\.(vue|ts|tsx|js|jsx|css|html)$/;

const PALETTE =
  'slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose';
const COLOUR_UTILITY =
  'bg|text|border|border-[trblxy]|ring|ring-offset|fill|stroke|outline|divide|from|via|to|shadow|accent|caret|decoration|placeholder';

const CHECKS: {
  pattern: RegExp;
  message: string;
  skip?: (lineText: string, path: string, index: number, match: string) => boolean;
}[] = [
  {
    pattern: /(?<![\w&#.])#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3,4})(?![\w-])/g,
    message: 'raw hex colour',
    // Vue slot shorthand (<template #add>) is not a colour.
    skip: (lineText, path) => (path.endsWith('.vue') || path.endsWith('.html')) && /<template\b[^>]*\s#/.test(lineText),
  },
  { pattern: /\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|light-dark|color-mix)\(/g, message: 'raw colour function' },
  {
    pattern: new RegExp(
      `(?<![\\w-])(?:[\\w-]+:)*(?:${COLOUR_UTILITY})-(?:(?:${PALETTE})-\\d{2,3}|white|black)(?![\\w-])`,
      'g',
    ),
    message: 'Tailwind palette colour instead of a token',
  },
  {
    pattern: /(?<![\w-])[a-z][\w-]*-\[[^\]\s]+\]/g,
    message: 'Tailwind arbitrary value instead of a token',
    // Arbitrary variants (data-[state=checked]:, aria-[sort=ascending]:) select a state and set no value. Reka UI
    // marks its parts' state with data-state, so these are how its components are styled.
    skip: (lineText, _path, index, match) => lineText[index + match.length] === ':',
  },
  { pattern: /(?<![\w\])-])\[[a-z-]+:[^\]\s]+\]/g, message: 'Tailwind arbitrary property instead of a token' },
  {
    pattern: /(?<![\w.#])(?<!\w-)-?(?!0+(?:\.0+)?(?:px|rem|em|pt)\b)\d*\.?\d+(?:px|rem|em|pt)\b/g,
    message: 'raw size',
  },
  {
    pattern:
      /\b(?:white|black|red|green|blue|orange|yellow|purple|pink|gray|grey|silver|navy|teal|maroon|olive|lime|aqua|fuchsia)\b(?=\s*[;!}])/g,
    message: 'named colour',
    skip: (_lineText, path) => !(path.endsWith('.css') || path.endsWith('.vue')),
  },
  {
    pattern: /\bline-height\s*:|(?<![\w-])(?:[\w-]+:)*(?:leading-[\w.]+|text-\d+\/[\w.]+)/g,
    message: 'line height (only the root has one, in the tokens file)',
  },
];

export const tokensOnly: Rule = {
  name: 'tokens-only',
  why: `Raw colours and sizes appear only in ${TOKENS_FILE}; the UI uses token utilities (bg-panel, text-13, rounded-lg).`,
  applies: (path) =>
    path.startsWith('ui/') && path !== TOKENS_FILE && SCANNED.test(path) && !path.includes('/node_modules/'),
  check(file) {
    const out: Violation[] = [];
    for (const { pattern, message, skip } of CHECKS) {
      for (const m of matches(file.text, pattern)) {
        if (skip?.(m.lineText, file.path, m.index, m.match)) continue;
        out.push({ rule: this.name, path: file.path, line: m.line, message: `${message} "${m.match}"` });
      }
    }
    return out;
  },
};
