// Rule: `claude` and `codex` are invoked only from the one fallback module. Enforced in its stricter form: no
// outside process is started anywhere in app code except that module, so a CLI call cannot hide behind a variable.

import { matches, type Rule, type Violation } from './types.ts';

export const FALLBACK_MODULE = 'crates/warden-core/src/fallback.rs';
const FALLBACK_DIR = 'crates/warden-core/src/fallback/';

const RUST_SPAWN =
  /\bCommand::new\b|\bprocess::Command\b|\bprocess::\{[^}]*\bCommand\b|\btokio::process\b|\basync_process\b|\bduct::|\bCreateProcess\w*|\bShellExecute\w*|\bWinExec\b|\btauri_plugin_shell\b|\btauri_plugin_opener\b|\bShellExt\b|\.sidecar\(|\bopen::that\w*/g;
const UI_SPAWN =
  /\bchild_process\b|\bexeca\b|\bDeno\.Command\b|@tauri-apps\/plugin-shell|@tauri-apps\/plugin-opener|\bCommand\.create\(/g;

function isRustAppCode(path: string): boolean {
  return (
    path.startsWith('crates/') &&
    path.endsWith('.rs') &&
    path !== FALLBACK_MODULE &&
    !path.startsWith(FALLBACK_DIR) &&
    // Integration tests and benchmarks may start the binaries they test.
    !/^crates\/[^/]+\/(tests|benches)\//.test(path)
  );
}

function isUiAppCode(path: string): boolean {
  return path.startsWith('ui/src/') && /\.(vue|[cm]?[jt]sx?)$/.test(path);
}

/** Drops `//` comments so doc comments may mention the APIs. Strings containing `//` are cut too, harmlessly. */
function stripLineComments(text: string): string {
  return text
    .split('\n')
    .map((line) => line.replace(/\/\/.*$/, ''))
    .join('\n');
}

export const cliFallback: Rule = {
  name: 'cli-fallback-only',
  why: `Only ${FALLBACK_MODULE} may start outside programs (claude, codex); CLI calls are a rare fallback, never on a timer. The UI starts none.`,
  applies: (path) => isRustAppCode(path) || isUiAppCode(path),
  check(file) {
    const pattern = file.path.endsWith('.rs') ? RUST_SPAWN : UI_SPAWN;
    const out: Violation[] = [];
    for (const m of matches(stripLineComments(file.text), pattern)) {
      out.push({
        rule: this.name,
        path: file.path,
        line: m.line,
        message: `starts a process ("${m.match}") outside the fallback module`,
      });
    }
    return out;
  },
};
