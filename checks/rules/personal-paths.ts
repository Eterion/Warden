// Rule: self-contained, and nothing personal or PC-specific in the repo. Every tracked or new file (not .git, not
// what .gitignore leaves out, not the lock files) is searched for:
//   - absolute drive and UNC paths (such as C:\<folder> or \\<server>\<share>), user folders in MSYS and Unix form
//   - the names of the person and machine running the check (user name, machine name, git user name and email)
//   - email addresses, except at reserved example domains
//   - URLs and internal hostnames, except loopback, reserved example domains and the links listed below
//   - `.mjs` helper script calls in app code
// Placeholders such as C:\Users\<name>, %USERPROFILE%, ${home} or {addr} name no one and are allowed.

import { execFileSync } from 'node:child_process';
import { hostname, userInfo } from 'node:os';
import { isGenerated, isRuleFixture, matches, type Rule, type Violation } from './types.ts';

/** The only outside hosts the repo may name. Anything else that leaks a host needs its own entry here. */
export const ALLOWED_HOSTS = ['api.anthropic.com', 'platform.claude.com'];

/** Links allowed by their start: documentation and fixed names that are not calls. */
export const ALLOWED_LINKS = [
  'https://github.com/Eterion/Warden', // Warden's own repository (README)
  'https://github.com/rsms/inter', // Inter's licence (ui/public/licenses/)
  'http://scripts.sil.org/OFL', // the SIL Open Font License, in the same file
  'https://www.conventionalcommits.org/', // the commit message rules (AGENTS.md)
  'https://api.openai.com/auth', // claim names inside Codex's login token, never called
  'https://api.openai.com/profile',
  'http://www.w3.org/2000/svg', // the SVG namespace, which every SVG file names (never fetched)
];

/** Reserved for examples (RFC 2606, RFC 6761) or this computer. */
function isExampleOrLocalHost(host: string): boolean {
  const h = host.toLowerCase().replace(/\.$/, '');
  return (
    ['localhost', '127.0.0.1', '[::1]', '0.0.0.0'].includes(h) ||
    /(^|\.)example\.(com|org|net)$/.test(h) ||
    /(^|\.)(example|test|invalid|localhost)$/.test(h)
  );
}

/** A placeholder rather than a name: <name>, %VAR%, ${var}, {var}, $VAR. */
const PLACEHOLDER = /[<{%$]/;

const DRIVE_PATH = /(?<![A-Za-z0-9])[A-Za-z]:(?:\\{1,2}|\/)[^\s"'`;,)]*/g;
// At the start of a word only, so a backslash written twice inside a string (C:\\<folder>) is not taken for one.
const UNC_PATH = /(?<=^|[\s"'`(=])(?:\\\\){1,2}[A-Za-z0-9][\w.-]*(?:\\\\|\\)[\w$.-]+/g;
const USER_FOLDER = [/(?<![\w.])\/[a-z]\/Users\/[^/\s"'`]+/gi, /(?<![\w.:/])\/(?:home|Users)\/[^/\s"'`]+/g];
const EMAIL = /[A-Za-z0-9._%+-]+@([A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+)/g;
const URL = /\b[a-z][a-z0-9+.-]*:\/\/[^\s"'`<>)\]]+/gi;
const INTERNAL_HOST = /(?<![\w.\\/-])[a-z0-9-]+(?:\.[a-z0-9-]+)*\.(?:local|lan|internal|intranet|corp|home\.arpa)\b/gi;
const HELPER_SCRIPT = /[\w-]+\.mjs\b/g;

/** Words too generic to count as someone's name. */
const GENERIC_NAMES = new Set(['user', 'users', 'admin', 'administrator', 'root', 'runner', 'localhost', 'default']);

/** The names of the person and the machine running the check. */
function localNames(): string[] {
  const names: string[] = [];
  const add = (v: string | undefined) => v && names.push(v.trim());
  try {
    add(userInfo().username);
  } catch {
    // No user name (some containers).
  }
  add(hostname());
  for (const key of ['user.name', 'user.email']) {
    try {
      const value = execFileSync('git', ['config', key], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] });
      add(value);
      if (key === 'user.name') value.split(/\s+/).forEach(add);
    } catch {
      // Not set.
    }
  }
  return names;
}

/** A pattern matching any of these names as a whole word, ignoring case; `null` when none is specific enough. */
export function namesPattern(names: string[]): RegExp | null {
  const words = [
    ...new Set(names.map((n) => n.trim()).filter((n) => n.length >= 3 && !GENERIC_NAMES.has(n.toLowerCase()))),
  ];
  if (!words.length) return null;
  const escaped = words.map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  return new RegExp(`(?<![\\w])(?:${escaped.join('|')})(?![\\w])`, 'gi');
}

function isAppCode(path: string): boolean {
  return path.startsWith('crates/') || path.startsWith('ui/src/');
}

function hostOf(url: string): string {
  const authority = url.replace(/^[a-z][a-z0-9+.-]*:\/\//i, '').split(/[/?#]/)[0] ?? '';
  const host = authority.slice(authority.lastIndexOf('@') + 1);
  return host.startsWith('[') ? host.slice(0, host.indexOf(']') + 1) : (host.split(':')[0] ?? '');
}

/** The rule, with the local names to look for (the running machine's by default). */
export function personalPathsRule(names: RegExp | null): Rule {
  return {
    name: 'personal-paths',
    why: 'Warden is self-contained and portable: nothing may name one person, their machine, their folders or private hosts, or call personal helper scripts.',
    applies: (path) => !isRuleFixture(path) && !isGenerated(path),
    check(file) {
      const out: Violation[] = [];
      const flag = (line: number, message: string) => out.push({ rule: this.name, path: file.path, line, message });

      for (const m of matches(file.text, DRIVE_PATH)) {
        if (!PLACEHOLDER.test(m.match)) flag(m.line, `absolute path "${m.match}"`);
      }
      for (const m of matches(file.text, UNC_PATH)) flag(m.line, `network path "${m.match}"`);
      for (const pattern of USER_FOLDER) {
        for (const m of matches(file.text, pattern)) {
          if (!PLACEHOLDER.test(m.match)) flag(m.line, `user folder "${m.match}"`);
        }
      }
      for (const m of matches(file.text, EMAIL)) {
        const domain = m.match.slice(m.match.lastIndexOf('@') + 1);
        if (!isExampleOrLocalHost(domain) && !ALLOWED_HOSTS.includes(domain.toLowerCase())) {
          flag(m.line, `email address "${m.match}" (use an example.com address)`);
        }
      }
      // A link listed one by one may name its owner (the repository's address names its account), so a name inside
      // one does not count; anywhere else it does.
      const listed: { line: number; from: number; to: number }[] = [];
      for (const m of matches(file.text, URL)) {
        const host = hostOf(m.match);
        if (!host || PLACEHOLDER.test(host)) continue;
        if (isExampleOrLocalHost(host) || ALLOWED_HOSTS.includes(host.toLowerCase())) continue;
        if (ALLOWED_LINKS.some((link) => m.match.startsWith(link))) {
          listed.push({ line: m.line, from: m.index, to: m.index + m.match.length });
          continue;
        }
        flag(m.line, `URL "${m.match}" (not an allowed host or link; see ALLOWED_LINKS)`);
      }
      for (const m of matches(file.text, INTERNAL_HOST)) flag(m.line, `internal hostname "${m.match}"`);
      if (names) {
        for (const m of matches(file.text, names)) {
          const inListed = listed.some((l) => l.line === m.line && m.index >= l.from && m.index + m.match.length <= l.to);
          if (!inListed) flag(m.line, `local user or machine name "${m.match}"`);
        }
      }
      if (isAppCode(file.path)) {
        for (const m of matches(file.text, HELPER_SCRIPT)) flag(m.line, `helper script "${m.match}" in app code`);
      }
      return out;
    },
  };
}

export const personalPaths: Rule = personalPathsRule(namesPattern(localNames()));
