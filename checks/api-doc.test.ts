// The API doc check catches a route, field or variant missing on either side. Run with `node --test checks/`.

import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  API_TYPES,
  DOC,
  ROUTES_FILE,
  check,
  docRoutes,
  docScope,
  jsonFields,
  jsonVariants,
  serverRoutes,
} from './api-doc.ts';

const RUST = `
    if method == Method::Post && path == "/api/events" {
    if method == "GET" && segments == ["state", "wait"] {
            ("GET", ["state"]) => {}
            (Method::Put, ["accounts", id, "hooks"]) => {
`;

const DOC_TEXT = `
| \`GET /api/state\`                       | | \`State\` |
| \`GET /api/state/wait?since=<revision>\` | | |
| \`POST /api/events?account={id}\`        | | |
| \`PUT /api/accounts/{id}/hooks\`         | \`{ routing?: boolean, status?: boolean }\` | |
`;

describe('api-doc', () => {
  it('reads the routes on both sides', () => {
    const routes = ['GET /api/state', 'GET /api/state/wait', 'POST /api/events', 'PUT /api/accounts/{id}/hooks'];
    assert.deepEqual(serverRoutes(RUST), [...routes].sort());
    assert.deepEqual(docRoutes(DOC_TEXT), [...routes].sort());
  });

  it('names fields and variants as serde writes them', () => {
    const rust = `
/// A thing.
#[derive(Serialize)]
#[serde(
    rename_all = "camelCase",
)]
pub struct Thing {
    pub started_at: i64,
    #[serde(skip)]
    hidden: u8,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(flatten)]
    pub place: Place,
}

#[derive(Serialize)]
struct Plain {
    snake_case: u8,
}

#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Why {
    /// Room.
    MainHasRoom,
    Expiring { resets_at: i64 },
    IdentityChanged {
        known_account_id: Option<String>,
    },
}

#[serde(rename_all = "lowercase")]
pub enum Kind {
    App,
    Cli,
}

#[serde(rename_all = "kebab-case")]
pub enum Shape {
    Session,
    OtherSignIn,
}
`;
    assert.deepEqual(jsonFields(rust, 'Thing'), ['startedAt', 'type']);
    assert.deepEqual(jsonFields(rust, 'Plain'), ['snake_case']);
    assert.equal(jsonFields(rust, 'Missing'), null);
    assert.deepEqual(jsonVariants(rust, 'Why'), [
      { name: 'mainHasRoom', fields: [] },
      { name: 'expiring', fields: ['resetsAt'] },
      { name: 'identityChanged', fields: ['knownAccountId'] },
    ]);
    assert.deepEqual(jsonVariants(rust, 'Kind'), [
      { name: 'app', fields: [] },
      { name: 'cli', fields: [] },
    ]);
    assert.deepEqual(jsonVariants(rust, 'Shape'), [
      { name: 'session', fields: [] },
      { name: 'other-sign-in', fields: [] },
    ]);
  });

  it("reads a type's part of the doc up to the next declaration, and an endpoint's row", () => {
    const doc = "```ts\ninterface A {\n  x: number;\n}\n\n// About B.\ntype B = 'b';\n```\n";
    assert.equal(docScope(doc, 'A'), 'interface A {\n  x: number;\n}\n');
    assert.equal(docScope(doc, 'B'), "type B = 'b';");
    assert.equal(docScope(DOC_TEXT, 'GET /api/state')?.includes('`State`'), true);
    assert.equal(docScope(DOC_TEXT, 'GET /api/nothing'), null);
  });

  it('catches a route, field or variant missing from the doc, even when another type has the same name', () => {
    const files: Record<string, string> = Object.fromEntries(API_TYPES.map((t) => [t.file, '']));
    files[ROUTES_FILE] = `${RUST}            (Method::Delete, ["accounts", id]) => {}\n`;
    files['crates/warden-core/src/model.rs'] =
      '#[serde(rename_all = "camelCase")]\npub struct Limit {\n    pub used_percent: f64,\n    pub resets_at: i64,\n}\n\n' +
      '#[serde(tag = "kind", rename_all = "camelCase")]\npub enum UsageError {\n    SignedOut,\n    NoLogs,\n    Failed { message: String },\n}\n';
    const doc =
      `${DOC_TEXT}| \`POST /api/gone\` | | |\n\n` +
      "```ts\ninterface Limit {\n  usedPercent: number;\n}\n\ntype UsageError = { kind: 'signedOut' } | { kind: 'failed' };\n\n" +
      'interface Other {\n  resetsAt: number;\n  message: string;\n}\n```\n';
    const messages = check(files, doc)
      .filter((p) => p.path === DOC)
      .map((p) => p.message);
    const has = (start: string) => messages.some((m) => m.startsWith(start));
    assert.ok(has('DELETE /api/accounts/{id} is answered'), 'undocumented route');
    assert.ok(has('POST /api/gone is in the endpoint table'), 'route not served');
    assert.ok(has('Limit.resetsAt'), 'field documented only on another type');
    assert.ok(!has('Limit.usedPercent'), 'documented field reported');
    assert.ok(has("UsageError 'noLogs'"), 'undocumented variant');
    assert.ok(has("UsageError 'failed'.message"), 'variant field documented only on another type');
    assert.ok(!has("UsageError 'signedOut'"), 'documented variant reported');
  });
});
