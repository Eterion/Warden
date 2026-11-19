// Rule: HTTP is only for the demo and the checks. Only the demo (crates/warden-server/examples/) and integration tests
// (crates/*/tests/) start the loopback server (`warden_server::start`), and only warden-server's lib.rs opens a
// listener, so real accounts are never served to a browser.

import { type Rule, type Violation } from './types.ts';

export const SERVER_LIB = 'crates/warden-server/src/lib.rs';

/** `warden_server::start` (or `crate::start` inside the crate) by path, or named in a `use` group. */
const START =
  /\b(?:warden_server|crate)::start\b|\b(?:warden_server|crate)::\{(?:[^{}]|\{[^{}]*\})*\bstart\b(?:[^{}]|\{[^{}]*\})*\}/g;
/** Anything that listens on the network. */
const LISTENER =
  /\btiny_http::Server\b|\bServer::https?\s*\(|\bTcpListener\b|\bUdpSocket\b|\bhyper::Server\b|\baxum::serve\b|\bwarp::serve\b|\bHttpServer::new\b/g;

function mayStart(path: string): boolean {
  return (
    path.startsWith('crates/warden-server/examples/') || /^crates\/[^/]+\/tests\//.test(path) || path === SERVER_LIB
  );
}

/** Drops `//` comments so doc comments may mention the APIs. Strings containing `//` are cut too, harmlessly. */
function stripLineComments(text: string): string {
  return text
    .split('\n')
    .map((line) => line.replace(/\/\/.*$/, ''))
    .join('\n');
}

/** Every match over the whole text (a `use` group may span lines), with the line it starts on. */
function found(text: string, pattern: RegExp): { line: number; match: string }[] {
  return [...text.matchAll(pattern)].map((m) => ({
    line: text.slice(0, m.index).split('\n').length,
    match: m[0].replace(/\s+/g, ' '),
  }));
}

export const httpDemoOnly: Rule = {
  name: 'http-demo-only',
  why: `HTTP is only for the demo and the checks: only the demo and integration tests start the loopback server (warden_server::start), and only ${SERVER_LIB} opens a listener. Real accounts are seen only by the desktop app.`,
  applies: (path) => path.startsWith('crates/') && path.endsWith('.rs'),
  check(file) {
    const text = stripLineComments(file.text);
    const out: Violation[] = [];
    if (!mayStart(file.path)) {
      for (const m of found(text, START)) {
        out.push({
          rule: this.name,
          path: file.path,
          line: m.line,
          message: `starts the loopback server ("${m.match}") outside the demo and integration tests`,
        });
      }
    }
    if (file.path !== SERVER_LIB) {
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
