# Warden

Warden is a Windows desktop app that watches your Claude and Codex accounts: how much of each limit is used, and which sessions are working, waiting for you or idle. Main is the Claude app's account; every other account is Warden's, in its own folder, with one switch that lets it take delegated work. A routing hook in main tells its AI where to run its work: main works itself until it reaches the share of its week you keep for your own work, which shrinks with the work hours left before its reset, and delegated work goes to the account whose allowance would otherwise go unused. Everything runs on your computer; the only outside calls are Anthropic's usage endpoint and its sign-in server, for Warden's own sign-in to main.

## Build and run

You need Rust 1.88 or later (1.95 for the desktop app), Node 24.2 or later, and Windows. Install the UI tooling once:

```sh
npm ci
```

**Browser version (for development).** Build the UI and the `warden` routing tool, then start the local server with a data folder of its own:

```sh
npm run build -w ui
cargo build -p warden-cli
cargo run -p warden-server -- --data-dir <folder> --ui-dir ui/dist
```

Open the address it prints (`http://127.0.0.1:4777/?token=...`): each run has its own token, and requests without it are refused. The routing hook runs the `warden.exe` built beside the server. For work on the UI, `npm run dev -w ui` serves it with instant reload and passes `/api/` to the server: open the same address with the dev server's port.

**Desktop app.** Build the UI and the routing tool, then run the app (`--data-dir` keeps a throwaway one apart from your own). It opens no network port: its window gets the page and the API in-process.

```sh
npm run build -w ui
cargo build -p warden-cli
cargo run -p warden-desktop -- --data-dir <folder>
```

`npm run desktop:installer` builds the installer into `target/release/bundle/nsis/`.

Without `--data-dir`, the server and the desktop app use your real data folder, `%APPDATA%\Warden`.

**After an update.** When Warden starts, it rewrites the hooks an earlier build wrote differently (another `warden.exe` or data folder, the old `curl` status hook, Claude's hooks before the exec form) in main and in the accounts switched on, and notes each repair in its log. A hook that is missing or not trusted shows in Settings, Accounts, with Repair. Claude's hooks need Claude Code 2.1.139 or later. Uninstalling Warden removes its hooks from every account, main included, restoring their backups.

**Demo.** `npm run demo` starts the real server on made-up accounts in a throwaway folder, with usage readings and sessions coming and going, so you can look around without touching a real account. Ctrl+C stops it and removes the folder.

## Checks

| What                                    | Command                 |
| --------------------------------------- | ----------------------- |
| Quick checks on what changed, as you go | `npm run check:quick`   |
| Full set on what changed                | `npm run check:changed` |
| Full set on everything                  | `npm run check`         |
| Format                                  | `npm run format`        |

The full set covers Rust (format, Clippy, tests including the rule tests and an end-to-end run of the installed hooks, performance budgets), the UI (types, lint, tests, contrast, and accessibility in light and dark with screenshots), and on every change the rule scripts and a check that `docs/api.md` names every route and field of the API.

## Rules

`AGENTS.md` holds the project's rules: the stack, the folders, how work is done and reviewed, and the rules every change follows (self-contained, login data, network, performance, UI). Read it before changing anything. The local server's API is in `docs/api.md`.
