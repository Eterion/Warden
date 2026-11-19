// Rule: HTTP is only for the demo and the checks. Only the demo (crates/warden-server/examples/) and integration tests
// (crates/*/tests/) start the loopback server (`warden_server::start`), and only warden-server's `start` in lib.rs
// opens a listener, so real accounts are never served to a browser.

import { type Rule, type Violation } from './types.ts';

export const SERVER_LIB = 'crates/warden-server/src/lib.rs';
const SERVER_SRC = 'crates/warden-server/src/';

/** Anything that listens on the network, named anywhere outside lib.rs. */
const LISTENER =
  /\btiny_http::Server\b|\bServer::https?\s*\(|\bTcpListener\b|\bUdpSocket\b|\bhyper::Server\b|\baxum::serve\b|\bwarp::serve\b|\bHttpServer::new\b/g;
/** Opening a listener in lib.rs, where `tiny_http::Server` is also the type `Running` holds. */
const OPENS_LISTENER =
  /\bServer::(?:https?|new|from_listener)\s*\(|\bTcpListener\b|\bUdpSocket\b|\bhyper::Server\b|\baxum::serve\b|\bwarp::serve\b|\bHttpServer::new\b/g;
/** A plain call of a function named `start`: not a method (`.start(`), a path (`x::start(`) or its definition. */
const BARE_START = /(?<![.:]\s*)(?<!\bfn\s+)\bstart\s*\(/g;

function mayStart(path: string): boolean {
  return path.startsWith('crates/warden-server/examples/') || /^crates\/[^/]+\/tests\//.test(path);
}

const IDENT = /[\p{L}\p{N}_]/u;

/**
 * Blanks Rust comments (line, block, nested block) and the insides of string and character literals with spaces,
 * keeping every newline and offset, so doc comments may mention the APIs, a `//` inside a string hides nothing after
 * it, and lines stay where they were.
 */
export function blankCommentsAndStrings(text: string): string {
  const out = text.split('');
  const blank = (from: number, to: number) => {
    for (let k = from; k < to && k < out.length; k++) if (out[k] !== '\n' && out[k] !== '\r') out[k] = ' ';
  };
  let i = 0;
  while (i < text.length) {
    const c = text[i];
    const prevIdent = i > 0 && IDENT.test(text[i - 1] ?? '');
    if (c === '/' && text[i + 1] === '/') {
      const end = text.indexOf('\n', i);
      const stop = end === -1 ? text.length : end;
      blank(i, stop);
      i = stop;
    } else if (c === '/' && text[i + 1] === '*') {
      let depth = 1;
      let j = i + 2;
      while (j < text.length && depth > 0) {
        if (text[j] === '/' && text[j + 1] === '*') {
          depth++;
          j += 2;
        } else if (text[j] === '*' && text[j + 1] === '/') {
          depth--;
          j += 2;
        } else j++;
      }
      blank(i, j);
      i = j;
    } else if (!prevIdent && /^[bc]?r#*"/.test(text.slice(i, i + 260))) {
      // A raw string: r"…", r#"…"#, br"…", cr"…"; no escapes, closed by the quote and as many #.
      const head = /^[bc]?r(#*)"/.exec(text.slice(i, i + 260));
      const hashes = head?.[1] ?? '';
      const open = i + (head?.[0].length ?? 0);
      const close = text.indexOf('"' + hashes, open);
      const end = close === -1 ? text.length : close;
      blank(open, end);
      i = close === -1 ? text.length : close + 1 + hashes.length;
    } else if (c === '"') {
      let j = i + 1;
      while (j < text.length && text[j] !== '"') j += text[j] === '\\' ? 2 : 1;
      blank(i + 1, j);
      i = j + 1;
    } else if (c === "'" && !prevIdent) {
      // A character literal ('x', '\n', '\'', '\u{1F600}', a non-BMP character); otherwise a lifetime or label.
      let end = -1;
      if (text[i + 1] === '\\') end = text.indexOf("'", i + 3);
      else if (text[i + 2] === "'" && text[i + 1] !== '\n') end = i + 2;
      else if (/[\uD800-\uDBFF]/.test(text[i + 1] ?? '') && text[i + 3] === "'") end = i + 3;
      if (end === -1) i++;
      else {
        blank(i + 1, end);
        i = end + 1;
      }
    } else i++;
  }
  return out.join('');
}

/** Every match over the whole text (a `use` group may span lines), with its offset and the line it starts on. */
function found(text: string, pattern: RegExp): { index: number; line: number; match: string }[] {
  return [...text.matchAll(pattern)].map((m) => ({
    index: m.index,
    line: text.slice(0, m.index).split('\n').length,
    match: m[0].replace(/\s+/g, ' '),
  }));
}

const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/**
 * The names `warden_server`'s root goes by in this file: the crate itself, inside it `crate`, `self` and `super`
 * (lib.rs is the parent of its modules), and every alias of one of those (`use warden_server as server`,
 * `extern crate warden_server as ws`, `use warden_server::{self as ws}`), followed through aliases of aliases.
 */
function roots(path: string, text: string): string[] {
  const names = new Set(['warden_server']);
  if (path.startsWith(SERVER_SRC)) for (const n of ['crate', 'self', 'super']) names.add(n);
  for (let grew = true; grew;) {
    grew = false;
    const alt = [...names].map(escape).join('|');
    const aliases = new RegExp(
      `\\b(?:use|extern\\s+crate)\\s+(?:::)?(?:${alt})\\s+as\\s+(\\w+)|\\b(?:${alt})::\\{[^{}]*?\\bself\\s+as\\s+(\\w+)`,
      'g',
    );
    for (const m of text.matchAll(aliases)) {
      const alias = m[1] ?? m[2];
      if (alias && alias !== '_' && !names.has(alias)) {
        names.add(alias);
        grew = true;
      }
    }
  }
  return [...names];
}

/** Where lib.rs's `pub fn start` body sits: from its opening brace to its closing one. */
function startBody(text: string): [number, number] | undefined {
  const m = /\bpub\s+fn\s+start\s*\(/.exec(text);
  if (!m) return undefined;
  const open = text.indexOf('{', m.index);
  if (open === -1) return undefined;
  let depth = 0;
  for (let j = open; j < text.length; j++) {
    if (text[j] === '{') depth++;
    else if (text[j] === '}' && --depth === 0) return [open, j];
  }
  return [open, text.length];
}

/** Every way this file reaches `warden_server::start`: by a path, in a `use` group, or called after a glob import. */
function startCalls(path: string, text: string): { index: number; line: number; match: string }[] {
  const alt = roots(path, text).map(escape).join('|');
  const byPath = new RegExp(
    `(?<![\\w:])(?:::)?(?:${alt})::start\\b|(?<![\\w:])(?:${alt})::\\{(?:[^{}]|\\{[^{}]*\\})*\\bstart\\b(?:[^{}]|\\{[^{}]*\\})*\\}`,
    'g',
  );
  const out = found(text, byPath);
  // Inside the crate `start` is in scope in lib.rs and reachable through `use super::*`; elsewhere through a glob.
  const glob = new RegExp(`(?<![\\w:])(?:${alt})::\\*`);
  if (path.startsWith(SERVER_SRC) || glob.test(text)) out.push(...found(text, BARE_START));
  return out;
}

export const httpDemoOnly: Rule = {
  name: 'http-demo-only',
  why: `HTTP is only for the demo and the checks: only the demo and integration tests start the loopback server (warden_server::start), and only its body in ${SERVER_LIB} opens a listener. Real accounts are seen only by the desktop app.`,
  applies: (path) => path.startsWith('crates/') && path.endsWith('.rs'),
  check(file) {
    const text = blankCommentsAndStrings(file.text);
    const out: Violation[] = [];
    if (!mayStart(file.path)) {
      for (const m of startCalls(file.path, text)) {
        out.push({
          rule: this.name,
          path: file.path,
          line: m.line,
          message: `starts the loopback server ("${m.match}") outside the demo and integration tests`,
        });
      }
    }
    if (file.path === SERVER_LIB) {
      const body = startBody(text);
      for (const m of found(text, OPENS_LISTENER)) {
        if (body && m.index > body[0] && m.index < body[1]) continue;
        out.push({
          rule: this.name,
          path: file.path,
          line: m.line,
          message: `opens a listener ("${m.match}") outside start in ${SERVER_LIB}`,
        });
      }
    } else {
      for (const m of found(text, LISTENER)) {
        out.push({
          rule: this.name,
          path: file.path,
          line: m.line,
          message: `opens a listener ("${m.match}") outside ${SERVER_LIB}`,
        });
      }
    }
    return out;
  },
};
