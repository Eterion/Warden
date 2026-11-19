// Each rule catches a planted violation and lets clean code through. Run with `node --test checks/`.

import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { checkFiles } from './run.ts';
import { namesPattern, personalPaths, personalPathsRule } from './personal-paths.ts';
import { TOKENS_FILE, tokensOnly } from './tokens-only.ts';
import { FALLBACK_MODULE, cliFallback } from './cli-fallback.ts';
import { SERVER_LIB, httpDemoOnly } from './http-demo-only.ts';
import type { Rule } from './types.ts';

function caught(rule: Rule, path: string, text: string): string[] {
  return checkFiles([{ path, text }], [rule]).map((v) => v.message);
}

describe('personal-paths', () => {
  it('catches user folders in every form', () => {
    assert.ok(
      caught(personalPaths, 'crates/warden-core/src/lib.rs', 'let p = "C:\\\\Users\\\\alice\\\\x";').length > 0,
      'planted violation not caught',
    );
    assert.ok(
      caught(personalPaths, 'ui/src/a.ts', "const p = 'C:/Users/alice/.claude';").length > 0,
      'planted violation not caught',
    );
    assert.ok(
      caught(personalPaths, 'AGENTS.md', 'see /c/Users/alice/notes').length > 0,
      'planted violation not caught',
    );
    assert.ok(caught(personalPaths, 'checks/x.ts', "'/home/alice/.codex'").length > 0, 'planted violation not caught');
  });

  it('catches .mjs helper calls in app code', () => {
    assert.ok(
      caught(personalPaths, 'crates/warden-cli/src/main.rs', 'run("node status.mjs near-limit")').length > 0,
      'planted violation not caught',
    );
    assert.ok(
      caught(personalPaths, 'ui/src/a.ts', "import x from './heavy.mjs'").length > 0,
      'planted violation not caught',
    );
  });

  it('catches absolute drive and network paths anywhere in the repo', () => {
    assert.ok(caught(personalPaths, 'README.md', 'Builds to D:\\builds\\warden').length > 0, 'drive path not caught');
    assert.ok(caught(personalPaths, 'checks/x.ts', "const p = 'E:/work/warden';").length > 0, 'drive path not caught');
    assert.ok(caught(personalPaths, 'docs/x.md', 'Shared at \\\\fileserver\\team').length > 0, 'UNC path not caught');
    assert.ok(
      caught(personalPaths, 'ui/src/a.ts', "const p = '\\\\\\\\nas01\\\\share';").length > 0,
      'UNC path not caught',
    );
  });

  it("catches the running person's and machine's names", () => {
    const local = personalPathsRule(namesPattern(['alice', 'ALICE-PC', 'Alice Example', 'alice@corp.example']));
    assert.ok(caught(local, 'docs/x.md', 'Measured on ALICE-PC').length > 0, 'machine name not caught');
    assert.ok(caught(local, 'checks/x.ts', '// ask Alice').length > 0, 'user name not caught');
    assert.deepEqual(
      caught(personalPathsRule(namesPattern(['al', 'user', 'admin'])), 'docs/x.md', 'all users and admins'),
      [],
    );
  });

  it('lets a listed link name its owner, and only there', () => {
    // The owner's login is public in the repository's address; `alice` stands for anyone else's name.
    const owner = personalPathsRule(namesPattern(['Eterion', 'alice']));
    assert.deepEqual(caught(owner, 'README.md', 'git clone https://github.com/Eterion/Warden.git'), []);
    assert.deepEqual(caught(owner, 'docs/x.md', 'https://github.com/Eterion/Warden/releases/latest'), []);
    assert.deepEqual(caught(owner, 'docs/x.md', 'Clone https://github.com/Eterion/Warden. Then build it.'), []);
    assert.deepEqual(caught(owner, 'docs/x.md', 'See https://www.conventionalcommits.org/en/v1.0.0/'), []);
    assert.deepEqual(caught(owner, 'docs/x.md', 'https://www.conventionalcommits.org/alice'), [
      'local user or machine name "alice"',
    ]);
    assert.ok(
      caught(owner, 'docs/x.md', 'https://github.com/Eterion/Warden.alice').includes(
        'local user or machine name "alice"',
      ),
      'name after a full stop inside the link not caught',
    );
    for (const text of [
      'https://github.com/Eterion/Warden/tree/alice',
      'https://github.com/Eterion/Warden/issues?q=author:alice',
      'https://github.com/Eterion/Warden|alice',
      'http://www.w3.org/2000/svg#alice',
    ]) {
      assert.ok(caught(owner, 'docs/x.md', text).includes('local user or machine name "alice"'), text);
    }
    assert.deepEqual(caught(owner, 'docs/x.md', 'https://github.com/Eterion/Warden-private'), [
      'URL "https://github.com/Eterion/Warden-private" (not an allowed host or link; see ALLOWED_LINKS)',
      'local user or machine name "Eterion"',
    ]);
    assert.deepEqual(caught(owner, 'README.md', '[its repository](https://github.com/Eterion/Warden)'), []);
    assert.ok(caught(owner, 'README.md', 'Made by Eterion').length > 0, 'name outside the link not caught');
    assert.ok(
      caught(owner, 'README.md', 'https://github.com/Eterion/Warden by Eterion').length > 0,
      'name beside the link not caught',
    );
    assert.deepEqual(caught(owner, 'README.md', 'https://github.com/Eterion/other'), [
      'URL "https://github.com/Eterion/other" (not an allowed host or link; see ALLOWED_LINKS)',
      'local user or machine name "Eterion"',
    ]);
  });

  it('catches email addresses and internal hosts or URLs', () => {
    assert.ok(
      caught(personalPaths, 'crates/x/src/lib.rs', 'let e = "jane.doe@company.com";').length > 0,
      'email not caught',
    );
    assert.ok(caught(personalPaths, 'docs/x.md', 'see https://wiki.company.com/warden').length > 0, 'URL not caught');
    assert.ok(caught(personalPaths, 'docs/x.md', 'see http://buildbox:8080/job').length > 0, 'internal URL not caught');
    assert.ok(caught(personalPaths, 'checks/x.ts', "const h = 'build01.corp';").length > 0, 'internal host not caught');
    assert.ok(caught(personalPaths, 'docs/x.md', 'on nas.local').length > 0, 'internal host not caught');
  });

  it('allows placeholders, example and loopback addresses, the usage endpoint and the listed links', () => {
    assert.deepEqual(caught(personalPaths, 'AGENTS.md', 'user folders such as C:\Users\<name> or %USERPROFILE%'), []);
    assert.deepEqual(caught(personalPaths, 'crates/warden-core/src/lib.rs', '// defaults: ~/.claude and ~/.codex'), []);
    assert.deepEqual(caught(personalPaths, 'ui/src/a.ts', 'const p = `${home}\\AppData\\Roaming`;'), []);
    assert.deepEqual(caught(personalPaths, 'ui/src/a.ts', "const e = ['ann@example.com', 'me@home.example'];"), []);
    assert.deepEqual(
      caught(personalPaths, 'ui/src/a.ts', "'http://127.0.0.1:4777', 'http://localhost:5173', `http://${host}`"),
      [],
    );
    assert.deepEqual(caught(personalPaths, 'crates/x/src/lib.rs', '"https://api.anthropic.com/api/oauth/usage"'), []);
    assert.deepEqual(caught(personalPaths, 'crates/x/src/lib.rs', '"https://platform.claude.com/v1/oauth/token"'), []);
    assert.deepEqual(
      caught(personalPaths, 'crates/x/src/lib.rs', '"https://chatgpt.com/backend-api/wham/rate-limit-reset-credits"'),
      [],
    );
    assert.ok(
      caught(personalPaths, 'crates/x/src/lib.rs', '"https://api.openai.com/v1"').length > 0,
      'other OpenAI host allowed',
    );
    assert.deepEqual(caught(personalPaths, 'ui/public/licenses/x.txt', 'http://scripts.sil.org/OFL'), []);
    assert.deepEqual(caught(personalPaths, 'icons/a.svg', '<svg xmlns="http://www.w3.org/2000/svg">'), []);
    assert.ok(
      caught(personalPaths, 'icons/a.svg', '<a href="http://www.w3.org/TR/SVG2/">').length > 0,
      'other w3.org link allowed',
    );
  });
});

describe('tokens-only', () => {
  const vue = 'ui/src/components/Thing.vue';

  it('catches raw colours', () => {
    assert.ok(
      caught(tokensOnly, vue, '<style>.a { color: #fafafa; }</style>').length > 0,
      'planted violation not caught',
    );
    assert.ok(
      caught(tokensOnly, 'ui/src/a.ts', "const c = 'rgba(0, 0, 0, 0.5)';").length > 0,
      'planted violation not caught',
    );
    assert.ok(
      caught(tokensOnly, vue, '<div class="bg-zinc-100 text-white">').length > 0,
      'planted violation not caught',
    );
    assert.ok(
      caught(tokensOnly, vue, '<style>.a { background: white; }</style>').length > 0,
      'planted violation not caught',
    );
  });

  it('catches raw sizes and arbitrary values', () => {
    assert.ok(
      caught(tokensOnly, vue, '<style>.a { margin: 13px; }</style>').length > 0,
      'planted violation not caught',
    );
    assert.ok(caught(tokensOnly, vue, '<div class="w-[13px] bg-[#fff]">').length > 0, 'planted violation not caught');
    assert.ok(
      caught(tokensOnly, vue, '<div class="[mask-type:luminance]">').length > 0,
      'planted violation not caught',
    );
    assert.ok(
      caught(tokensOnly, 'ui/src/a.ts', "el.style.width = '1.5rem';").length > 0,
      'planted violation not caught',
    );
    assert.ok(
      caught(tokensOnly, vue, '<style>.a { margin-top: -4px; }</style>').length > 0,
      'planted violation not caught',
    );
  });

  it('catches per-element line heights', () => {
    assert.ok(caught(tokensOnly, vue, '<p class="text-13/6">').length > 0, 'planted violation not caught');
    assert.ok(caught(tokensOnly, vue, '<p class="leading-tight">').length > 0, 'planted violation not caught');
    assert.ok(
      caught(tokensOnly, vue, '<style>p { line-height: 1.2; }</style>').length > 0,
      'planted violation not caught',
    );
  });

  it('allows token utilities, slot shorthand and the tokens file', () => {
    assert.deepEqual(
      caught(tokensOnly, vue, '<div class="bg-panel text-secondary rounded-lg p-4 text-13 border-divider">'),
      [],
    );
    assert.deepEqual(caught(tokensOnly, vue, '<template #add><span /></template>'), []);
    assert.deepEqual(
      caught(
        tokensOnly,
        vue,
        '<span class="data-[state=checked]:bg-switch-on aria-[sort=ascending]:text-text text-secondary/50">',
      ),
      [],
    );
    assert.deepEqual(caught(tokensOnly, 'ui/src/a.ts', 'const n = this.#abc + this.#feed;'), []);
    assert.deepEqual(caught(tokensOnly, vue, '<style>.a { margin: 0px; width: var(--spacing-bar); }</style>'), []);
    assert.deepEqual(caught(tokensOnly, TOKENS_FILE, '--color-win: light-dark(#f4f4f5, #18181b);'), []);
    assert.deepEqual(caught(tokensOnly, 'crates/x/src/lib.rs', 'let c = "#ffffff";'), []);
  });
});

describe('cli-fallback-only', () => {
  it('catches processes started outside the fallback module', () => {
    const rs = 'crates/warden-core/src/usage.rs';
    assert.ok(
      caught(cliFallback, rs, 'let out = Command::new("claude").arg("-p").output();').length > 0,
      'planted violation not caught',
    );
    assert.ok(
      caught(cliFallback, rs, 'use std::process::{Command, Stdio};').length > 0,
      'planted violation not caught',
    );
    assert.ok(
      caught(cliFallback, rs, 'let c = std::process::Command::new(exe);').length > 0,
      'planted violation not caught',
    );
    assert.ok(
      caught(cliFallback, rs, 'tokio::process::Command::new("codex")').length > 0,
      'planted violation not caught',
    );
    assert.ok(
      caught(cliFallback, 'ui/src/a.ts', "import { spawn } from 'node:child_process';").length > 0,
      'planted violation not caught',
    );
    assert.ok(
      caught(cliFallback, 'ui/src/a.ts', "import { Command } from '@tauri-apps/plugin-shell';").length > 0,
      'planted violation not caught',
    );
    assert.ok(
      caught(cliFallback, rs, 'use std::{io, process::{Command as Run}};').length > 0,
      'planted violation not caught',
    );
    assert.ok(caught(cliFallback, rs, 'app.shell().sidecar("x")').length > 0, 'planted violation not caught');
    assert.ok(
      caught(cliFallback, 'ui/src/a.mts', "import cp from 'node:child_process';").length > 0,
      'planted violation not caught',
    );
  });

  it('allows the fallback module, integration tests and comments', () => {
    assert.deepEqual(caught(cliFallback, FALLBACK_MODULE, 'Command::new("claude")'), []);
    assert.deepEqual(caught(cliFallback, 'crates/warden-core/src/fallback/sessions.rs', 'Command::new("codex")'), []);
    assert.deepEqual(
      caught(cliFallback, 'crates/warden-cli/tests/route.rs', 'Command::new(env!("CARGO_BIN_EXE_warden"))'),
      [],
    );
    assert.deepEqual(caught(cliFallback, 'crates/warden-core/src/lib.rs', '//! Never use Command::new here.'), []);
  });
});

describe('http-demo-only', () => {
  const desktop = 'crates/warden-desktop/src/shell.rs';

  it('catches the loopback server started from the desktop app', () => {
    assert.ok(
      caught(httpDemoOnly, desktop, 'let running = warden_server::start(config, 4787)?;').length > 0,
      'planted violation not caught',
    );
    assert.ok(
      caught(httpDemoOnly, 'crates/warden-server/src/app.rs', 'let r = crate::start(config, 0);').length > 0,
      'planted violation not caught',
    );
  });

  it('catches a use of start', () => {
    assert.ok(caught(httpDemoOnly, desktop, 'use warden_server::start;').length > 0, 'planted violation not caught');
    assert.ok(
      caught(httpDemoOnly, desktop, 'use warden_server::{\n    Config,\n    start,\n};').length > 0,
      'planted violation not caught',
    );
    assert.ok(
      caught(httpDemoOnly, desktop, 'use warden_server::{app::{App}, start as serve};').length > 0,
      'planted violation not caught',
    );
  });

  it('catches a listener outside lib.rs', () => {
    assert.ok(
      caught(httpDemoOnly, desktop, 'let l = std::net::TcpListener::bind("127.0.0.1:0")?;').length > 0,
      'planted violation not caught',
    );
    assert.ok(
      caught(httpDemoOnly, 'crates/warden-server/src/http.rs', 'tiny_http::Server::http("127.0.0.1:0")').length > 0,
      'planted violation not caught',
    );
    assert.ok(
      caught(httpDemoOnly, 'crates/warden-server/examples/demo.rs', 'let s = Server::http(addr)?;').length > 0,
      'planted violation not caught',
    );
    assert.ok(
      caught(httpDemoOnly, 'crates/warden-cli/tests/route.rs', 'let l = TcpListener::bind(addr);').length > 0,
      'planted violation not caught',
    );
  });

  it('allows the demo, integration tests, lib.rs itself, start_core and comments', () => {
    assert.deepEqual(
      caught(
        httpDemoOnly,
        'crates/warden-server/examples/demo.rs',
        'use warden_server::{Config, start};\nstart(c, 0);',
      ),
      [],
    );
    assert.deepEqual(
      caught(httpDemoOnly, 'crates/warden-server/tests/common/mod.rs', 'use warden_server::{Config, Running, start};'),
      [],
    );
    assert.deepEqual(
      caught(httpDemoOnly, SERVER_LIB, 'let server = tiny_http::Server::http(SocketAddr::new(BIND_IP, port));'),
      [],
    );
    assert.deepEqual(caught(httpDemoOnly, desktop, 'use warden_server::{Config, start_core};'), []);
    assert.deepEqual(
      caught(httpDemoOnly, desktop, '//! Unlike warden_server::start, no TcpListener is opened here.'),
      [],
    );
  });
});
