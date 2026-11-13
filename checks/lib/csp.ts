// The page's Content-Security-Policies, read from where the app sets them, so the rendered check runs the page
// under the same ones: the development server's (crates/warden-server/src/http.rs) and the desktop window's
// (crates/warden-desktop/tauri.conf.json).

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { ROOT } from './repo.ts';

/** The policy the development server sends with every answer (`CSP` in http.rs). */
export function browserCsp(): string {
  const source = readFileSync(join(ROOT, 'crates', 'warden-server', 'src', 'http.rs'), 'utf8');
  const m = /pub const CSP: &str =\s*"([^"]+)";/.exec(source);
  if (!m?.[1]) throw new Error('the development server has no CSP constant in http.rs');
  return m[1];
}

/** The policy the desktop window's answers carry (`app.security.csp` in tauri.conf.json). */
export function desktopCsp(): string {
  const config = JSON.parse(readFileSync(join(ROOT, 'crates', 'warden-desktop', 'tauri.conf.json'), 'utf8')) as {
    app?: { security?: { csp?: unknown } };
  };
  const csp = config.app?.security?.csp;
  if (typeof csp !== 'string' || !csp) throw new Error('tauri.conf.json has no app.security.csp');
  return csp;
}
