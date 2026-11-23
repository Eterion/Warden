// Keeps docs/api.md in step with the server: every route the server answers is in the doc's endpoint table and
// every route there is answered; every field of the types the API sends or takes is named in its documented type
// (or its endpoint's row, for a request body), and every variant of the API's enums, with its fields, is too.
//
//   node checks/api-doc.ts

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { ROOT } from './lib/repo.ts';

export const DOC = 'docs/api.md';
/** The API's routes and request bodies, answered in-process or over HTTP. */
export const ROUTES_FILE = 'crates/warden-server/src/api.rs';
/** Routes only the demo server answers over HTTP (the status hook's events). */
export const HTTP_FILE = 'crates/warden-server/src/http.rs';

/**
 * A Rust type the API sends or takes, and where the doc describes it: TypeScript types by name (a struct written
 * inline in another type names that one), or an endpoint's row (`PUT /api/routing`) for a request body.
 */
export interface ApiType {
  file: string;
  type: string;
  kind: 'struct' | 'enum';
  in: string[];
}

const APP = 'crates/warden-server/src/app.rs';
const MODEL = 'crates/warden-core/src/model.rs';
const SESSIONS = 'crates/warden-core/src/sessions.rs';
const MEMORY = 'crates/warden-core/src/memory.rs';
const s = (file: string, type: string, ...where: string[]): ApiType => ({ file, type, kind: 'struct', in: where });
const e = (file: string, type: string, ...where: string[]): ApiType => ({ file, type, kind: 'enum', in: where });

export const API_TYPES: ApiType[] = [
  s(APP, 'SettingsPatch', 'Settings', 'PUT /api/settings'),
  s(APP, 'IdentityView', 'Identity'),
  s(APP, 'IdentityChangeView', 'IdentityChange'),
  s(APP, 'AccountView', 'Account'),
  s(APP, 'MainView', 'Main'),
  e(APP, 'Problem', 'Problem'),
  s(ROUTES_FILE, 'MainBody', 'PATCH /api/main'),
  s('crates/warden-core/src/mainlogin.rs', 'SignIn', 'SignIn'),
  e('crates/warden-core/src/mainlogin.rs', 'SignInState', 'SignInState'),
  s(APP, 'SettingsView', 'Settings'),
  s(APP, 'DesktopView', 'State'),
  s(APP, 'StateView', 'State'),
  s(APP, 'DelegationView', 'Delegation'),
  s(APP, 'DelegatedView', 'Delegation'),
  s(APP, 'ActivityView', 'State'),
  s(APP, 'Candidate', 'Candidate'),
  s(ROUTES_FILE, 'AddBody', 'POST /api/accounts'),
  s(ROUTES_FILE, 'NewBody', 'POST /api/accounts/new'),
  s(ROUTES_FILE, 'PatchBody', 'PATCH /api/accounts/{id}'),
  s(ROUTES_FILE, 'IdentityBody', 'POST /api/accounts/{id}/identity'),
  s(ROUTES_FILE, 'SetupBody', 'POST /api/setup'),
  s(MODEL, 'Limit', 'Limit'),
  s(MODEL, 'ModelLimit', 'Usage'),
  s(MODEL, 'Extra', 'Usage'),
  s(MODEL, 'Usage', 'Usage'),
  e(MODEL, 'Provider', 'Provider'),
  e(MODEL, 'UsageError', 'UsageError'),
  e('crates/warden-core/src/hooks.rs', 'Setup', 'HookSetup'),
  e('crates/warden-core/src/store.rs', 'Theme', 'Theme'),
  s(SESSIONS, 'Session', 'Session'),
  s(SESSIONS, 'Place', 'Session'),
  s(SESSIONS, 'Subagent', 'Session'),
  e(SESSIONS, 'SessionState', 'SessionState'),
  e(SESSIONS, 'Source', 'Session'),
  s(SESSIONS, 'Bin', 'Bin'),
  s(MEMORY, 'MemoryDay', 'MemoryDay'),
  s(MEMORY, 'Summary', 'MemoryDay'),
  s(MEMORY, 'SeriesSummary', 'MemoryDay'),
  s(MEMORY, 'Gap', 'MemoryDay'),
  s(MEMORY, 'Series', 'MemorySeries'),
  s(MEMORY, 'SeriesInfo', 'MemorySeries'),
  e(MEMORY, 'Kind', 'MemoryKind'),
  s(MEMORY, 'Minute', 'MemoryMinute'),
];

export interface Problem {
  path: string;
  message: string;
}

const segment = (seg: string): string => {
  const t = seg.trim();
  return t.startsWith('"') ? t.slice(1, -1) : `{${t}}`;
};

/** `GET /api/accounts/{id}`-style routes the server answers (`Method::Get` or `"GET"`). */
export function serverRoutes(rust: string): string[] {
  const routes = new Set<string>();
  const add = (method: string | undefined, path: string) => routes.add(`${(method ?? '').toUpperCase()} ${path}`);
  // A method is written `Method::Get` or `"GET"`.
  const method = String.raw`(?:Method::(\w+)|"([A-Z]+)")`;
  const all = (pattern: string) => rust.matchAll(new RegExp(pattern, 'g'));
  for (const m of all(String.raw`\(${method},\s*\[([^\]]*)\]\)\s*=>`)) {
    const parts = (m[3] ?? '')
      .split(',')
      .filter((p) => p.trim())
      .map(segment);
    add(m[1] ?? m[2], `/api/${parts.join('/')}`);
  }
  for (const m of all(String.raw`method == ${method} && path == "(\/api\/[^"]+)"`)) add(m[1] ?? m[2], m[3] ?? '');
  for (const m of all(String.raw`method == ${method} && segments == \[([^\]]*)\]`))
    add(m[1] ?? m[2], `/api/${(m[3] ?? '').split(',').map(segment).join('/')}`);
  return [...routes].sort();
}

/** The routes in the doc's endpoint table, without their query strings. */
export function docRoutes(doc: string): string[] {
  const routes = new Set<string>();
  for (const m of doc.matchAll(/^\|\s*`(GET|POST|PUT|PATCH|DELETE) (\/api\/[^`?\s]*)/gm)) routes.add(`${m[1]} ${m[2]}`);
  return [...routes].sort();
}

/**
 * The part of the doc that describes `where`: an endpoint's table row, or a TypeScript type from its declaration
 * to the next top-level line (another declaration, a comment or the end of the code block). `null`: not there.
 */
export function docScope(doc: string, where: string): string | null {
  const lines = doc.split('\n');
  if (/^[A-Z]+ \//.test(where)) {
    const [method, path] = where.split(' ');
    const row = lines.find((l) => l.startsWith('|') && new RegExp(`^\\|\\s*\`${method} ${path}[\`?]`).test(l));
    return row ?? null;
  }
  const start = lines.findIndex((l) => new RegExp(`^(interface|type) ${where}\\b`).test(l));
  if (start < 0) return null;
  const end = lines.findIndex((l, i) => i > start && /^(interface |type |\/\/|```)/.test(l));
  return lines.slice(start, end < 0 ? undefined : end).join('\n');
}

const camel = (n: string): string => n.replace(/_([a-z0-9])/g, (_, c: string) => c.toUpperCase());

/** A Rust type's body and the attributes right above it (an attribute may span lines: `key = "value",`, `)]`). */
function item(rust: string, keyword: 'struct' | 'enum', type: string): { attrs: string; body: string } | null {
  const start = rust.search(new RegExp(`\\b${keyword} ${type}\\s*\\{`));
  if (start < 0) return null;
  const above = rust.slice(0, start).split('\n');
  above.pop();
  let attrs = '';
  const ATTR_LINE = /^\s*(#\[|\/\/|\)\]|[a-z_]+\s*=)/;
  while (above.length && ATTR_LINE.test(above.at(-1) ?? '')) attrs = `${above.pop() ?? ''}\n${attrs}`;
  return { attrs, body: rust.slice(rust.indexOf('{', start) + 1, rust.indexOf('\n}', start)) };
}

/** Fields of a struct body (or an enum variant's), as serde names them; `skip` and `flatten` fields left out. */
function fieldsOf(body: string, camelCase: boolean): string[] {
  const out: string[] = [];
  let pending = '';
  for (const raw of body.split('\n')) {
    const line = raw.trim();
    if (line.startsWith('#[')) {
      pending += line;
      continue;
    }
    const m = /^(?:pub(?:\([^)]*\))?\s+)?([a-z_][a-z0-9_]*)\s*:/.exec(line);
    if (!m) continue;
    const attr = pending;
    pending = '';
    if (/serde\([^)]*\b(skip|flatten)\b/.test(attr)) continue;
    const renamed = /serde\([^)]*\brename\s*=\s*"([^"]+)"/.exec(attr);
    const name = m[1] ?? '';
    out.push(renamed?.[1] ?? (camelCase ? camel(name) : name));
  }
  return out;
}

/** The JSON names of a Rust struct's fields, as serde writes them (`rename_all = "camelCase"`, `rename`, `skip`, `flatten`). */
export function jsonFields(rust: string, type: string): string[] | null {
  const it = item(rust, 'struct', type);
  return it && fieldsOf(it.body, /rename_all\s*=\s*"camelCase"/.test(it.attrs));
}

/** An enum's variants as serde writes them (`rename_all` camelCase, lowercase or kebab-case), with their fields (`rename_all_fields`). */
export function jsonVariants(rust: string, type: string): { name: string; fields: string[] }[] | null {
  const it = item(rust, 'enum', type);
  if (!it) return null;
  const all = /rename_all\s*=\s*"([\w-]+)"/.exec(it.attrs)?.[1];
  const fieldsCamel = /rename_all_fields\s*=\s*"camelCase"/.test(it.attrs);
  const rename = (v: string) =>
    all === 'lowercase'
      ? v.toLowerCase()
      : all === 'camelCase'
        ? v[0]?.toLowerCase() + v.slice(1)
        : all === 'kebab-case'
          ? v.replace(/(?<=.)([A-Z])/g, '-$1').toLowerCase()
          : v;
  const out: { name: string; fields: string[] }[] = [];
  // Variants are the lines at depth 0 starting with a capital; `Name { a: T }` on one line or `Name {` ... `},`.
  let open: { name: string; body: string } | null = null;
  for (const raw of it.body.split('\n')) {
    const line = raw.trim();
    if (open) {
      if (line.startsWith('}')) {
        out.push({ name: rename(open.name), fields: fieldsOf(open.body, fieldsCamel) });
        open = null;
      } else open.body += `${line}\n`;
      continue;
    }
    const m = /^([A-Z]\w*)\s*(\{([^}]*)\}|\{)?/.exec(line);
    if (!m) continue;
    const name = m[1] ?? '';
    if (m[2] === '{') open = { name, body: '' };
    else out.push({ name: rename(name), fields: fieldsOf((m[3] ?? '').split(',').join('\n'), fieldsCamel) });
  }
  return out;
}

/** Every route served but not documented (and the other way round), and every API field or variant the doc does not name. */
export function check(files: Record<string, string>, doc: string): Problem[] {
  const problems: Problem[] = [];
  const served = serverRoutes([ROUTES_FILE, HTTP_FILE].map((f) => files[f] ?? '').join('\n'));
  const documented = docRoutes(doc);
  for (const r of served.filter((r) => !documented.includes(r)))
    problems.push({ path: DOC, message: `${r} is answered by the server but not in the endpoint table` });
  for (const r of documented.filter((r) => !served.includes(r)))
    problems.push({ path: DOC, message: `${r} is in the endpoint table but the server does not answer it` });
  // A field counts as named when its type's part of the doc has it as a TypeScript field (`name:`, `name?:`) or in
  // a body shape (`{ name, ... }`); a variant, when it has it as a string (`'name'`).
  const field = (f: string) => new RegExp(`(^|[\\s{,(])${f}\\??(:|,|\\s*\\})`, 'm');
  for (const t of API_TYPES) {
    const scopes = t.in.map((w) => docScope(doc, w));
    const missing = t.in.filter((_, i) => scopes[i] === null);
    if (missing.length) {
      problems.push({ path: DOC, message: `${missing.join(', ')} (for ${t.type}) is not in the doc` });
      continue;
    }
    const scope = scopes.join('\n');
    const where = t.in.join(' or ');
    const rust = files[t.file] ?? '';
    const notFound = { path: t.file, message: `${t.kind} ${t.type} not found (update API_TYPES in checks/api-doc.ts)` };
    if (t.kind === 'struct') {
      const fields = jsonFields(rust, t.type);
      if (fields === null) problems.push(notFound);
      for (const f of (fields ?? []).filter((f) => !field(f).test(scope)))
        problems.push({ path: DOC, message: `${t.type}.${f} (from ${t.file}) is not in ${where}` });
    } else {
      const variants = jsonVariants(rust, t.type);
      if (variants === null) problems.push(notFound);
      for (const v of variants ?? []) {
        if (!scope.includes(`'${v.name}'`))
          problems.push({ path: DOC, message: `${t.type} '${v.name}' (from ${t.file}) is not in ${where}` });
        for (const f of v.fields.filter((f) => !field(f).test(scope)))
          problems.push({ path: DOC, message: `${t.type} '${v.name}'.${f} (from ${t.file}) is not in ${where}` });
      }
    }
  }
  return problems;
}

function main(): void {
  const read = (p: string) => readFileSync(join(ROOT, p), 'utf8');
  const paths = [...new Set([ROUTES_FILE, HTTP_FILE, ...API_TYPES.map((t) => t.file)])];
  const problems = check(Object.fromEntries(paths.map((p) => [p, read(p)])), read(DOC));
  if (problems.length === 0) {
    console.log(`api-doc: ${DOC} names every route, API field and variant`);
    return;
  }
  for (const p of problems) console.error(`${p.path}: ${p.message}`);
  console.error(`\napi-doc: ${problems.length} problem(s); update ${DOC} to match the server`);
  process.exitCode = 1;
}

if (import.meta.main) main();
