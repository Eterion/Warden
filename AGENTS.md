# Warden: instructions for agents

Read this before writing or reviewing any code here. It is the one place the project's rules are written down; Claude and Codex both follow it, and the reviewer agents judge against it.

Warden is a Windows desktop app that watches the user's Claude and Codex accounts and tells AI sessions where to run their work.

**Local working files.** What Warden does and why is in `warden-plan.md`; each build step has a brief in `briefs/`; `design/` is the design's starting point (read `design/README.md` first). They are local and untracked (`.gitignore`), not part of the repo: code, checks, tests and docs work and make sense without them, and every rule that lasts is written here.

## How work is done

- **A step builds only what its brief asks.** Nothing from the plan's Later section, no extra features, no speculative abstractions. When something is needed that the brief does not cover, stop and ask.
- Each brief's "Stop and ask" list is binding. Also stop and ask before changing the stack, a rule here, or a rule in the plan.
- For a screen not built yet, where `design/` and the plan disagree, the plan wins; ask before guessing. For screens that exist, the app and the rules under UI are the reference.
- Never touch the user's real Claude or Codex account folders (`~/.claude`, `~/.codex`, any `CLAUDE_CONFIG_DIR` or `CODEX_HOME`) beyond what the brief allows; test against copies.

## Stack

- Rust (edition 2024; 1.88 or later for let-chains and the `time` crate, 1.95 for the desktop app's `windows-registry`) for the core, the local server and the `warden` CLI.
- Vue 3 + Vite + TypeScript, Reka UI (unstyled components), Tailwind CSS 4, Lucide icons only.
- VueUse (`@vueuse/core`, at the major version Reka UI uses) for browser APIs: visibility, event listeners with cleanup, intervals, click outside, reduced motion. No Pinia, vue-router or es-toolkit.
- Tauri 2 (`crates/warden-desktop`) around the same core, API and UI. Windows only. The window loads the page and calls the API through the app's own URL scheme, answered in-process (no network port); the page asks Tauri itself only for its window's controls, the folder picker and Quit. Updates come through Tauri's updater, driven from Rust (`tauri-plugin-updater`).
- Node is for the UI tooling and the check scripts (`checks/`, plain TypeScript run directly by Node), never part of the running app.

## Folders

```
crates/warden-core     core library: accounts, usage, rules, policy; fallback.rs is the only place that starts programs
crates/warden-server   the core's API (docs/api.md), answered in-process or by the development server on 127.0.0.1; examples/demo.rs is the demo server
crates/warden-cli      the `warden` binary: `warden route` and `warden refresh`, which the routing hook runs
crates/warden-desktop  the desktop app (Tauri): window, tray, start with Windows, notifications
crates/warden-rules    test harness for Warden's own rules (dev-dependency only)
ui/                    Vue app; ui/src/styles/tokens.css holds every design token
checks/                check command, rule scripts, API doc check, contrast, accessibility, budgets, review helpers, the demo
.agents/reviewers/     reviewer instructions, shared by Claude (.claude/agents/) and Codex (checks/review-codex.ts)
.github/workflows/     the release workflow: builds, signs and publishes a release on a version tag
artifacts/             generated: screenshots, budget numbers, review inputs (ignored by git)
```

## Commands

Run from the repo root.

| What                                                                                                                                                  | Command                                              |
| ----------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------- |
| Install                                                                                                                                               | `npm ci` (Rust needs only `cargo`)                   |
| UI with instant reload                                                                                                                                | `npm run dev -w ui`                                  |
| Development server (`127.0.0.1:4777`; open the address it prints, with its token, or that address on the UI's dev server, which passes `/api/` to it) | `cargo run -p warden-server -- --data-dir <folder>`  |
| Desktop app (finds the built UI, `npm run build -w ui`; `--data-dir` for a throwaway one beside your own)                                             | `cargo run -p warden-desktop -- --data-dir <folder>` |
| Desktop installer (`target/release/bundle/nsis/`)                                                                                                     | `npm run desktop:installer`                          |
| Quick checks, only what changed                                                                                                                       | `npm run check:quick`                                |
| Full set, only what changed                                                                                                                           | `npm run check:changed`                              |
| Full set, everything                                                                                                                                  | `npm run check`                                      |
| Rule scripts alone                                                                                                                                    | `npm run rules`                                      |
| Demo on made-up accounts (live data, nothing real read; Ctrl+C stops it)                                                                              | `npm run demo`                                       |
| Format with Prettier (all, or the files named)                                                                                                        | `npm run format`                                     |
| Make a release (only when the user asks)                                                                                                              | see `docs/releasing.md`                              |

- Without `--data-dir`, the server and the desktop app use the real data folder, `%APPDATA%\Warden`. The routing hook runs the `warden.exe` beside them: `cargo build -p warden-cli` first.
- Development builds only (`cargo run`, never the installer): `--no-hooks` makes Warden write no hooks anywhere (the live checks run on the real accounts with it; the window says hooks are off), and `--home <folder>` uses a test home in place of `%USERPROFILE%` (for the hook checks). Release builds refuse both.
- **One Claude hook** (`.claude/settings.json`): after Edit, Write or MultiEdit, `checks/format-edited.ts` formats the edited file with Prettier, or rustfmt for `.rs`. It is quick and never fails the turn. There are no other hooks; checks run when you run them. Codex and other editors format with `npm run format` and `cargo fmt`.
- `check:quick` while working. It skips the slow checks (rendered accessibility with screenshots, performance budgets) and anything for an area that did not change: Rust checks only when Rust changed, UI checks only when the UI changed. The rule scripts and the API doc check always run.
- Run `npm run check` once per step, before review. After review fixes, use `check:changed -- --base <the commit the fixes started from>`; do not repeat the full set.
- In `check:changed`, budgets run for production Rust, build inputs, desktop configuration, the UI build the window loads, or changes to the budget harness, numbers or its local imports (`checks/budgets.ts`, `checks/budgets.json`, `checks/lib/pipe.ts`, `checks/lib/repo.ts`). Other tests, docs and check scripts alone skip them. UI test-only changes also skip the rendered check. Otherwise rendered accessibility visits the states affected by the changed screen's imports, in both themes. App shell and settings-frame imports count for every state; shared or unknown inputs fall back to every state. Its CSP helper (`checks/lib/csp.ts`) also triggers the rendered check. `npm run check` always runs every check and every state.
- Rust tests first compile with `cargo test --workspace --quiet --no-run`, without a time limit, then run in parallel with `cargo test --workspace --quiet` under a five-minute limit. UI test execution also has a five-minute limit. Timeout failures report Vitest's running tests or Rust's "has been running for over 60 seconds" notices, otherwise say the names are unknown. They stop the command's process tree and return on exit or within a one-second grace period, closing output pipes even if an orphan holds them open.
- "Changed" means since `--base <ref>` (default `master`), including uncommitted and untracked files; `--dry-run` shows what would run. The default covers only what is not committed to `master` yet; once a step has commits, pass `--base <the commit the step started from>`.

What the full set runs:

- Rust: `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test` (including the rule tests), the performance budgets (release build).
- UI: `vue-tsc`, ESLint (code problems only; `eslint-config-prettier` leaves formatting to Prettier), Vitest, WCAG AA contrast on the token pairs, axe (WCAG 2.2 AA) on the rendered pages in light and dark, with screenshots in `artifacts/screenshots/`. The rendered check drives the Edge that ships with Windows; nothing is downloaded.
- Always: the rule scripts, the API doc check (`checks/api-doc.ts`: the server's routes and API fields match `docs/api.md`), `prettier --check` (TS, JS, Vue, CSS, JSON, YAML, Markdown; on the changed files only in `check:changed` and `check:quick`), and in the full set the commit messages (Conventional Commits, on the commits since `--base`).
- The check scripts' own tests and types.

Never weaken a check to make it pass: no ignored or deleted tests, no loosened rule scripts or allowlists, no disabled lints, no raised budgets without the user's approval. If a check is wrong, say so and ask.

## Rules

From the plan (Self-contained and portable, Performance, Platform, Review). Each sits in the cheapest place that enforces it; the rest is for you and the reviewers.

**Self-contained**

- Everything happens inside the app. No calls to personal scripts or helper files (the user's `.mjs` scripts and the like). Folders and accounts are detected on first run or picked in Settings.
- Nothing personal or PC-specific anywhere in the repo (code, tests, docs, config): no absolute drive or UNC paths, no user or machine names, no real email addresses, no internal hostnames or URLs. Use placeholders (`%USERPROFILE%` for paths in test data, fixtures, screenshots and docs, such as `%USERPROFILE%\.claude`; `C:\Users\<name>` only where a drive path itself is the point; `${home}`), `example.com` addresses and hosts, and loopback. The only outside hosts named are `api.anthropic.com`, `platform.claude.com`, `chatgpt.com` and the GitHub hosts the release download uses (`github.com`, `objects.githubusercontent.com`, `release-assets.githubusercontent.com`); other links (documentation, licences, and the SVG namespace `http://www.w3.org/2000/svg` that SVG files need) are listed one by one in `ALLOWED_LINKS` in the rule script, the repository's own address (`https://github.com/Eterion/Warden`) among them; a name inside a listed link does not count as naming someone. Keep vector sources as SVG; PNG and ICO files are generated from them (the desktop icons from `crates/warden-desktop/icons/warden.svg`). _Rule script: `personal-paths`_ (every tracked or new file except the lock files; it also looks for the user, machine and git names of whoever runs it).
- The only outside programs the app runs are the official `claude` and `codex` CLIs, found on PATH, and Explorer for the desktop app's "Open log folder". Hooks only run `warden.exe` (`warden event`, `warden route`, `warden refresh`; the uninstaller runs `warden unhook`): Claude's in its exec form (no shell), Codex's through `cmd.exe /C`, which Codex always uses.
- Only `crates/warden-core/src/fallback.rs` (or a `fallback/` folder beside it) may start a process; the UI starts none. Integration tests (`crates/*/tests/`) may start the binaries they test. _Rule script: `cli-fallback-only`._ If another process start seems needed, stop and ask.
- The app's own data (settings, history, and its small rolling log in `logs`: one file a day, a few days, capped in total) lives in its data folder. The log never holds a token, prompt, tool input or message (the rule tests read it too).

**Login data**

- A Claude account's login token is read only to call Anthropic's usage endpoint (the request identifies as the installed Claude CLI, by its version, so the answer also lists the account's limit resets), and identity (email, organisation) only to tell accounts apart. Tokens are never logged, shown or sent anywhere else, and never kept longer than the call. Warden never reads the Claude app's stored login (`~/.claude`'s `.credentials.json`) and never renews or writes the login in any account folder. _Rule tests: `tokens_never_reach_logs_or_responses`, `account_logins_are_never_written`._
- A Codex account's access token and account id are read from its `auth.json` only to ask ChatGPT's backend for the account's banked resets (`chatgpt.com`): one request with a usage reading every 3 hours, and whenever usage is refreshed by hand; never logged, shown, saved or sent anywhere else, and not kept beyond the call. _Rule test: `tokens_never_reach_logs_or_responses`._
- One exception: Warden's own sign-in to main, in its own folder in the data folder (`main-sign-in`), made with the official `claude` sign-in. Warden reads it to read main's usage and renews it itself when it expires, sending its refresh token to Anthropic's sign-in server, and keeps the renewed login only in that folder. Only the process holding the data folder's server lock renews it. Each renewal is logged (when it holds until), and a refused one once; never the login itself. It too is never logged, shown or sent anywhere else.

**Accounts and sign-ins**

- Main is always the Claude desktop app's account, its folder `~/.claude`: Warden reads who it is signed in as and writes its routing and status hooks there; it cannot be switched off, removed or swapped. Its usage comes from Warden's own sign-in (Login data).
- Every other account is Warden's: a Claude or Codex account in its own folder (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`), added in Add account (a new account, whose folder Warden creates under `~/.warden` as `claude-<name>` or `codex-<name>` before opening the official sign-in, or an existing folder, found there or as `~/.claude-*` and `~/.codex-*`) or ticked in first-run setup. Its one switch, takes delegated work, installs Warden's status hook when on and removes it (with the backup) when off. Warden accounts never get the routing hook. Remove asks first and removes the hook too.
- Warden never renews a Warden account's login. While one has expired, its last reading still counts (a weekly reset that has passed counts as 0%), and its CLI renews it the next time the account works.
- Uninstalling Warden removes its hooks from every account, main included, restoring the backups.
- The Codex desktop app's folder (`~/.codex`) is never an account: Warden never reads or writes anything in it, and the server refuses it however it is written (`..`, 8.3 short names, links, junctions, quotes, case, `CODEX_HOME`). Warden only knows whether the Codex app is installed. _Rule test: `the_codex_apps_folder_is_left_alone`._
- Settings › General holds only the app's own settings. Warden finds the apps and CLIs it needs on its own (CLIs on PATH; no CLI path setting) and says so only where one is missing.
- A folder signed in as someone other than its account (main or a Warden account): its sessions show the real sign-in, count for no account (now chip, sparkline) and get no routing, until the change is accepted or undone.

**Network**

- The desktop app opens no network port: its window gets the page and the API in-process, through the app's own URL scheme.
- HTTP is for development only (the server binary, `npm run dev`, the demo, the checks). The development server listens only on 127.0.0.1, checks the Host header, and accepts requests only from this computer and the app's own page (Origin checked; no Origin is fine; a browser's `Sec-Fetch-Site` only `same-origin` or `none`), and only with its random per-run token: the address it prints carries it, the first request trades it for a cookie. _Rule tests: `server_binds_to_loopback_only`, `server_listens_on_loopback_only`, `server_checks_the_host_header`, `server_checks_the_origin`, `server_checks_the_fetch_site`, `server_checks_the_token`._
- A strict Content-Security-Policy on every answer: own scripts, styles, fonts and images only, nothing inline, connections only to the app itself (the desktop window's in `tauri.conf.json`, also Tauri's `ipc:` bridge). The rendered accessibility check runs the pages under both and fails on anything blocked; the budgets check fails if the real window does not load its page, styles and API. _Rule tests: `server_sends_a_strict_csp`, `the_window_policy_is_strict`._
- The only outside network calls are the usage endpoint, Anthropic's sign-in server (to renew Warden's own sign-in to main), ChatGPT's backend for Codex accounts' banked resets, and GitHub, where the installed app looks for and downloads its updates (`warden_core::policy::ALLOWED_OUTBOUND_HOSTS`: `api.anthropic.com`, `platform.claude.com`, `chatgpt.com`, and the GitHub hosts the release download uses: `github.com`, `objects.githubusercontent.com`, `release-assets.githubusercontent.com`; the update download follows redirects only to them). _Rule test: `only_the_usage_endpoint_the_sign_in_server_codex_resets_and_github_releases_are_allowed`._
- Hooks reach the core only over the per-user named pipe (`warden_core::pipe`): a security descriptor for the current user alone, a fresh name each start written to the data folder, and `warden event` sending only to the process that file names. Hooks never use the HTTP port.

**Event-driven**

- Hooks push events; nothing re-reads everything on a timer. The status hook's events keep only the event's name, session id, working folder, transcript path, tool names, subagent ids and types, and the notification kind, and the parent session id (`WARDEN_PARENT_SESSION`, which Warden's delegation command sets, checked like a session id) (`warden_core::pipe::HookEvent`): `warden event` drops the rest (prompts, tool inputs and outputs, messages) before sending, and the core refuses a message with more. _Rule test: `prompts_and_tool_inputs_never_kept`._ Sessions end when their CLI's process exits (Windows tells Warden; no polling). A Claude transcript (its path comes with the event; only inside the account's folder, no links followed) is read only for its newest `custom-title` record, or without one its newest `ai-title` (Claude Code's own generated title), and Codex's `session_index.jsonl` only for its thread names: each from where it was last read, on events and file-change notifications, keeping only the title. _Rule test: `transcripts_read_only_for_their_title`._ Usage is read on its interval (one small request per Claude account; every 3 hours, and whenever usage is refreshed by hand, it also asks for the account's limit resets, and one more request per Codex account for its banked resets; `warden refresh` never asks for them; main's sign-in is renewed only when a read finds it expired); Codex logs are read only from where they were last read, on file-change notifications.
- No CLI calls on a timer: each `claude` or `codex` call starts a whole process, so the CLIs' session lists are a rare fallback.
- The installed app looks for an update when it starts and once an hour (one small request); development builds never do. It downloads a newer version on its own and installs it only on "Install and restart" (confirmed first) or when Warden quits.

**Performance**

- The core is small and always on; the window exists only while open. Fixed-size, capped history in memory; compact storage on disk.
- Hooks add no noticeable delay: the status hook runs in the background and `warden event` gives up within a second; `warden route` answers from saved readings and starts in milliseconds.
- No heavy chart or UI libraries; UI updates pause while the window is hidden; nothing animates in the background.
- Budgets (memory and CPU at idle, hook times, binary sizes) are tests in `checks/budgets.ts` with numbers in `checks/budgets.json`. A new budget is `null` (measured, never failing) until the step that first measures it proposes a number for the user to approve.

## Rule test harness

`crates/warden-rules` holds the checks; `crates/warden-server/tests/rules.rs` applies them to the real code, including the running server (listener address, Host, Origin, `Sec-Fetch-Site`, tokens, outbound hosts, what the status hook keeps). New rules a test can check go into the harness first, with a test showing it catches a violation.

## UI

### What the app looks like

The app itself is the design. `ui/src/styles/tokens.css` and the shared components below are the living design system; the light and dark screenshots the full check writes to `artifacts/screenshots/` show what the app looks like now, and the UX reviewer compares against them and the rules here.

**Look**

- A modern desktop app in the style of Tailwind's Catalyst kit: zinc neutrals, Inter, black, white and greys. Light and dark from one set of tokens, following Windows or the user's choice.
- Colour only for status; there is no accent colour. Amber means it needs you or is near a limit. Usage bars are green below 50%, orange from 50%, red from 80%, and neutral grey while a Warden account's login is idle (expired).
- Every text, icon and control colour meets WCAG AA in both themes: 4.5:1 for text, 3:1 for icons, bars (against their track and what is around it) and control outlines.
- 8px corners on controls, cards, tables and the content panel; 16px on modals; 1px borders.
- Lucide icons only. Provider icons are the plain marks (Claude's orange spark, Codex's mark in the text colour).

**App-wide rules**

- The primary button comes first in every button group (Windows order: "Save" then "Cancel").
- No hint text that only repeats what a menu, heading or label already says.
- No dots as separators: use layout, gaps, a thin divider, a tag, or commas in running text.
- One unitless line height (1.5) on the root; no per-element line heights.
- The UI is in English whatever the Windows region is: dates and numbers use `Intl` with `'en'` and a 24-hour clock.
- Tables and lists sit in one bordered box with faint row dividers; settings rows are divided without a box.
- Custom switches, segmented controls, chips and sliders; sliders instead of typing wherever a value is a number (times of day too); radio cards for a choice that needs a mark and a line of text (Claude or Codex in Add account); a segmented control for a choice between a few short options (New account or Existing folder).
- Every field that takes a folder has a "Choose folder…" button beside it that opens the standard Windows folder picker in the desktop app (`pickFolder`); the development browser version keeps the text field and hides the button.
- Switches: off is a hollow outlined track with a small knob, on is a filled track with a large knob, so each state reads on its own.
- A switch or slider that cannot be changed says why (`disabledReason`): it stays focusable with `aria-disabled` (not `disabled`), shows the reason in the shared tooltip on hover and focus, and does nothing when activated. Only its visuals fade, never its focus ring.
- Every interactive element has a hover and a pressed state: buttons, icon buttons, switches, checkboxes, radios, chips, radio cards, segmented controls, menu items, nav items, rows that act, links, the window's caption buttons and slider thumbs. Neutral controls light with `chip-off` or `active` and press with `press`; filled ones (primary, on) go to `primary-hover` and `primary-press`; rows and cards take the faint `row-hover` and `row-press`; a switch that is off darkens its track, outline and knob, and a slider thumb darkens. Pressed states never scale anything (the modal and menu animations are not pressed states). Disabled controls show neither.
- A label and its control react as one: hovering or pressing a connected label (`for`/`id`, or around the control) shows the control's hover or pressed state. Where a setting's title names its switch or checkbox, it is that control's `<label>`.
- Modals and menus animate open and closed: a soft scale (from 0.96, menus 0.97) with a mild overshoot, about 200 ms opening (menus 150 ms) and quicker closing, the backdrop fading. With reduced motion they only fade, quickly. Nothing else animates, except the working spinners.
- Every spinner shows the same angle at any moment (`v-sync-spin`: the animation starts at the timeline's zero; no timers).
- The main window's content uses the available width, inside the panel's padding.
- Only the app's own areas scroll (the content panel, the sidebar in a short window, a modal's body); the page itself never does (the rendered check fails if it can). Every scrolling area has thin scrollbars whose thumb is the `control` colour (3:1), set once in the base styles.
- Menus hug their items (as wide as the longest, within the window). Every menu item has a Lucide icon; destructive ones (Remove) are red (`danger`), passing contrast at rest, highlighted and pressed.
- A changed sign-in shows amber "Signed in as …" in place of the account's email line (the expected email in its tooltip), and in the Account column of its sessions.
- A switched-off account shows only in Settings › Accounts: a dimmed icon, a grey name with the "Not used" tag, and its row a subtle grey background (`WTableRow` `off`, the `row-off` token).
- Settings › Accounts says something about an account's sign-in, usage or CLI only when something is wrong: in amber, with its fix beside it (Sign in, Accept, Undo, Scan again). Nothing while all is well.
- Its Hooks column always shows each account's hooks: a check with "Installed"; amber "Not trusted", "Missing" or "Still installed" with Repair beside it; grey "Not installed" on a switched-off account; grey "Off for this run" with `--no-hooks`.
- The status bar names one account, the one that gets the work now: "Work goes to Main until 98%" or "Work goes to Claude alt", its reason in the tooltip. Then "Hooks are off for this run" in a `--no-hooks` run, and "Usage read N min ago".
- Main's hand-off point (where it starts delegating, 100% less the reserve now) shows in the status bar, as a mark on main's weekly bar on the Overview, and as "Hands off at 98% now" under the reserve slider.
- A Warden account whose login has expired shows no amber message: its bars turn grey, with the reading's age and a tooltip saying its login is idle. Main's expired sign-in stays amber, with Sign in again.
- Extra usage shows only once some is spent; the Overview's Extra usage column shows only while some account has spent some.
- Limit resets (Claude's limit resets and Codex's banked resets) show only while an account in use has one: a list on the right of the Overview (under it in a narrow window), one row per reset, each with its account's icon and name, its short name ("Full reset", "5-hour reset") and when it expires, soonest first; an expired one is gone. One that expires within the warning days set in Settings › General ("Warn before a reset expires", 3 days unless changed) shows its expiry in amber, with an amber hourglass and how soon ("Expires in 2 days, …", "Expires tomorrow, …", "Expires today, …").
- Account icon badges: a crown on main; no other badge.
- Work sent to the other accounts is "delegated work" everywhere.
- A session's name is plain text: its own title, else its branch in words, else its folder. No work-item tags or numbers taken out of it.
- The sessions table is a tree: each delegated session sits under the main session that started it, indented with a thin connector, always expanded; waiting rows come first among siblings; a session whose parent is unknown or gone stays at the top level; while accounts are checked, the ancestors of their sessions show faded in their places. An ended or not-known session's whole row shows faded, as a faded parent does (text tertiary, icons muted, no amber, the provider mark in its own colour). Ended and not-known sessions can be hidden with the "Ended and not known" checkbox at the right end of the Sessions header, beside the counts (its label with their count, shown while there is one), and the choice is remembered; a hidden one still shows as the faded parent of a shown session.

**States and icons**

- Working: a slow, neutral grey spinner, a full thin ring with a darker arc turning on it (1.8 s a turn; still when reduced motion is asked for), in the sidebar and tables alike.
- Waiting for you: the Lucide hand in amber.
- Idle: the Lucide coffee cup.
- Done: a dash, muted.
- Not known (seen only in a CLI's list): a ghost, muted.

### Shared components

New screens reuse these instead of restyling their own (all in `ui/src/components/`):

| Component          | What it is                                                                                                                                                                                                         |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `AppModal`         | Every modal: focus kept inside and given back on close, Escape and a click beside it close it (`escapeOnly`: first-run setup; `blocking`: neither, while an update installs), `stacked` for a second modal on top. |
| `WSwitch`          | The switch (the only on/off control); with `disabledReason` it stays focusable (`aria-disabled`) and says why in the tooltip.                                                                                      |
| `SegmentedControl` | One choice from a few, as a radio group, optionally with icons.                                                                                                                                                    |
| `MoreMenu`         | The "more actions" menu on a row: items with an icon, `danger` for destructive ones; drawn into the modal it sits in.                                                                                              |
| `WMenuContent`     | A dropdown menu's panel: hugs its items within the window, animates open and closed. Holds `WMenuItem`s (icon, label, `danger`).                                                                                   |
| `AccountIcon`      | An account's provider mark, with the crown on main.                                                                                                                                                                |
| `ProviderMark`     | Claude's and Codex's plain marks in the shared sizes.                                                                                                                                                              |
| `UsageBar`         | A usage bar on the panel or the window background, coloured by level, grey while a login is idle (`idle`); an optional `mark` (main's hand-off point).                                                             |
| `StateIcon`        | A session state's icon: the working spinner, the amber hand, the coffee cup, done, not known.                                                                                                                      |
| `NowChip`          | An account's current sessions as state icons and counts, amber when one waits for you.                                                                                                                             |
| `WSlider`          | The slider (every number set by dragging): thin track, dark fill, round thumb; `range` for two thumbs (work hours); saves once per drag (`commit`); `disabledReason`.                                              |
| `WTooltip`         | A short explanation on hover and keyboard focus (instead of `title`), around a plain button trigger or, `as-child`, the caller's.                                                                                  |
| `WButton`          | A button with text: primary, outline, quiet (`md` among chips, `sm` on the window background) or link; an optional icon; `busy`.                                                                                   |
| `WIconButton`      | A button that is only an icon, named by its label: `md` beside a heading, `sm` on a row; also a Reka trigger through `as-child`.                                                                                   |
| `WTextField`       | A one-line text field: a name, a path (mono), or the search box with its magnifier. The caller sets the width.                                                                                                     |
| `ToggleChip`       | A chip that is on or off (`aria-pressed`): filled with a check, or outline for a pick from a list.                                                                                                                 |
| `RemovableChip`    | A chip a click removes, named by that action: `md` for an item the user added, `sm` for a filter in force.                                                                                                         |
| `RadioCard`        | A radio card inside a Reka `RadioGroupRoot`: a mark, title, text and a round check; `disabledReason`.                                                                                                              |
| `WTag`             | A small grey pill beside an account's name ("Not used").                                                                                                                                                           |
| `NavItem`          | An item in the sidebar or the Settings nav: icon and label, filled when current; a link with `href`, otherwise a button.                                                                                           |
| `WList`            | A plain list in one bordered box; its `WListItem` rows have faint dividers, and with `action` a row is a button (`select`).                                                                                        |
| `WTable`           | A table: one bordered box, a header row and `WTableRow` rows with faint dividers on a shared grid; `on="panel"` scrolls sideways.                                                                                  |

The Overview's own parts sit there too: `SubagentsBadge` (a session's subagents, their types on hover and focus) and `ActivitySparkline` (the last 2 hours of working sessions, no chart library).

### In code

- **Tokens only.** `ui/src/styles/tokens.css` is the only place raw colours and sizes appear. Tailwind's own palette, font sizes, radii, line heights and shadows are switched off, so only token utilities exist: colours `bg-panel`, `text-secondary`, `border-divider`, `bg-attn-bg`; type `text-10` to `text-20` and `font-normal` to `font-bold`; radii `rounded-xs`, `-sm` (tags), `-md` (small buttons and chips), `-lg`, `-2xl` and `-full`; spacing on the 4px scale (`p-2` = 8px, `gap-1.5` = 6px, `h-9` = 36px); the `shadow-*` tokens (`shadow-active`, `shadow-modal`, ...); `animate-spin-slow`. _Rule script: `tokens-only`_ (no hex, `rgb()`, palette classes, arbitrary values `w-[13px]`, raw `px`/`rem` or line heights, `leading-*` or `text-13/6`, outside the tokens file; ESLint forbids static inline styles). Arbitrary _variants_ that select a state are fine: `data-[state=checked]:bg-switch-on` is how Reka UI parts are styled.
- **Hover and pressed**: write them with the `hov:` and `press:` variants (`group-hov:`, `group-press:` for a part inside a `group` control), never bare `hover:` or `active:`. They are `:hover` and `:active`, plus the state a connected label passes on through `v-label-state` (`ui/src/components/labels.ts`), and skip disabled controls. Put `v-label-state` on any control that can have a label (the shared switch and text field have it). They add no specificity, so a state's own colour wins unless stacked on it (`aria-pressed:hov:`). Each new hover or pressed colour gets its contrast pairs.
- **Themes.** Each colour token holds `light-dark(light, dark)`. The theme follows Windows; `<html data-theme="light|dark">` overrides it. Never branch on the theme in components.
- **Adding a token**: reuse an existing token where one fits; otherwise add it to `tokens.css` (for a screen not built yet, take the value from `design/`'s `light:` and `dark:` maps). If it is a text, icon, bar or control colour, add its pairs to `ui/src/styles/contrast-pairs.json` (4.5 for text, 3 for icons, bars and control outlines; a bar against its track as well as the background). The contrast check fails on any pair below WCAG AA.
- Inter ships with the app as a Latin-subset variable woff2 with its licence (SIL OFL 1.1). Lucide icons only (`lucide-vue-next`).
- Pages and states the rendered accessibility check visits are listed in `checks/a11y.ts` (`PAGES`, with the steps that reach a state); add each new screen and state there, so it also gets light and dark screenshots for the UX reviewer.

## Commits, review and the step summary

1. Everything is committed straight to `master`; there are no other branches. Note the commit the step starts from (`git rev-parse HEAD` before its first commit). Commit at meaningful points: a feature finished and approved, a bug fixed, a step landed with its checks passed; neither one giant commit nor a commit for every experiment the user may undo after seeing it. An agent run on a build step still commits working states as it goes, so a later run can continue, and the step's commits land together. `master` is pushed to the GitHub repository (`origin`) on its own at those meaningful points, with its checks passed; never force-pushed. Releases are made only when the user asks. Versions stay `0.MINOR.PATCH` (0.1.0, 0.2.0, 0.2.1) until the user says Warden is stable.
   - Commits are authored with the user's GitHub no-reply address, which their git config sets for this folder; nothing personal goes into a commit message.
   - Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/): `type(scope): summary`, with the type one of `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert`, and the scope the area when there is one (`core`, `server`, `cli`, `ui`, `checks`). The summary says what changed, in plain words; the body says why when it is not obvious. _Check: `checks/commits.ts`_ (in `check:changed` and `check`, on the commits since `--base`).
2. Before review: run `npm run check` once for the step and make it pass. Use `check:quick` while working.
3. `npm run review:prep -- --base <the commit the step started from>` writes the step's change to `artifacts/review/` for Claude's reviewer agents, including `plan.json` with the reviews needed (`review:codex` writes it itself).
4. The other model reviews; the writer never reviews its own code.
   - Choose by risk: only rendered UI source (`ui/src/`, minus tests) and assets (`ui/public/`) are UX-only. Other `ui/` paths, including dev security and configuration, need code review; `ui/index.html` and `ui/vite.config.ts` need UX too. Production Rust and unknown build inputs need code, and UX too when rendered UI also changes. Other tests, docs and check scripts need no review, except the enforcement files below.
   - Files that enforce the rules always keep code review, even when they are tests, docs or check scripts: `AGENTS.md` and `CLAUDE.md` at any depth, `checks/rules/**`, `checks/budgets.ts`, `checks/budgets.json`, `checks/lib/review.ts`, `checks/lib/selection.ts`, `checks/lib/pages.ts`, `checks/run.ts`, `checks/a11y.ts`, `checks/contrast*`, `ui/src/styles/contrast-pairs.json`, `crates/warden-server/tests/rules.rs`, `crates/warden-rules/**`, `.agents/**` and `.claude/**`. Rendered UI enforcement files also get UX review.
   - Codex wrote it: start the needed Claude `code-reviewer` and `ux-reviewer` agents together, in parallel when both are needed, each in a fresh context, given the brief's path.
   - Claude wrote it: `npm run review:codex -- auto --brief briefs/<step>.md --base <the commit the step started from>` chooses by risk and starts the needed read-only Codex sessions in parallel; reports land in `artifacts/review/`. Explicit `code` and `ux` commands remain available.
   - Both reviewers' instructions are in `.agents/reviewers/`.
5. Fix each finding or answer it, commit the fixes, and run `npm run check:changed -- --base <the commit the fixes started from>`. The step's full check is not repeated after review fixes.
   - A run with a one-off brief in `briefs/` (for a review, a fix or a polish pass) deletes that brief once its work is done; the build-step briefs stay until the build ends.
6. On landing, the run's passing check result stands after a fast-forward or a rebase that leaves its final files unchanged. If the rebase changes the final files, rerun checks for those changes (`check:changed` from the pre-rebase commit); do not rerun just because commit hashes changed.
7. Each brief names the files and the sections of this file it needs read first. End with a short summary for the user to read afterwards (a report, not a gate): what changed, the commits, the check results and performance numbers, the reviewers' findings and what was fixed or answered, open questions.
