# Warden API

The desktop app answers this API in-process, with no network port: its window loads the page from the app's own URL scheme (`http://warden.localhost/` in WebView2) and sends `/api/` requests there. A request with an `Origin` is accepted only from that page.

Over HTTP the API is answered only in the demo and the checks, always on made-up data. The demo server listens on `http://127.0.0.1:4787` (`npm run demo -- --port <port>` to change it) and nowhere else, and starts each run with a random token: it prints `open http://127.0.0.1:<port>/?token=<token>`. Opening that address answers `303` to the same page without the token, setting the cookie `warden-token-<port>` (`HttpOnly`, `SameSite=Strict`); every request without that cookie gets `403`. Instant reload (`npm run dev -w ui`) works only with the demo on its default port, 4787: Vite's proxy sends `/api/` there and its cookie is `warden-token-4787`. Open the demo's address with Vite's port in place of 4787; Vite trades the token for that cookie and passes it on with `/api/`. Every request must also carry a `Host` of `127.0.0.1:<port>` or `localhost:<port>`; a request with an `Origin` is accepted only from `http://127.0.0.1:<port>` or `http://localhost:<port>` (no `Origin` is fine), and a browser's `Sec-Fetch-Site`, when sent, must be `same-origin` or `none` (so another web page cannot reach even the GET routes without an `Origin`). Anything else gets `403`.

Every answer carries a strict `Content-Security-Policy`: the page's own scripts, styles, fonts and images only, nothing inline, and requests only to the app itself (in the desktop app also Tauri's bridge to its window, `ipc:`). The desktop window's policy is `app.security.csp` in `crates/warden-desktop/tauri.conf.json`; the demo server's is `CSP` in `crates/warden-server/src/http.rs`.

Development builds of the desktop app accept `--no-hooks` (record hook actions without writing account folders or taking backups; `hooksOff: true` shows the notice) and `--home <folder>` (use a test home for account folders, ignoring inherited account-folder overrides). Their routing hooks pass `--home` to `warden route` and `warden refresh` in debug builds only, and a debug `warden unhook` takes `--home` too, so the hook checks clean the test home's main instead of the real one. Release desktop and CLI builds refuse these development switches before starting.

The hooks do not use this API: they reach the core over a per-user pipe (see the status hook below).

Bodies are JSON (`Content-Type: application/json`). Times are milliseconds since the Unix epoch. Errors are `{ "error": "<plain sentence>" }` with a 4xx or 5xx status: `400` for a body not in the expected shape (unknown fields included), `404` for an unknown route or account id, `413` for a body over 64 KB. No response ever contains a login token.

Besides `/api/`, the built UI (`ui/dist`) is served: by the desktop app from the UI it finds, by the demo server from its `--ui-dir <folder>`; any other method there gets `405`.

## Types

```ts
type Provider = 'claude' | 'codex';

interface Identity {
  email: string;
  organization: string | null;
}

interface Limit {
  usedPercent: number;
  resetsAt: number | null;
}

interface Usage {
  readAt: number;
  fiveHour: Limit | null;
  weekly: Limit | null;
  perModelWeekly: { model: string; limit: Limit }[];
  extra: {
    enabled: boolean;
    usedCredits: number | null;
    monthlyLimit: number | null;
    usedPercent: number | null;
  } | null;
}

type UsageError =
  | { kind: 'signedOut' }
  | { kind: 'loginExpired'; status?: number }
  | { kind: 'identityChanged' }
  | { kind: 'noLogs' }
  | { kind: 'failed'; message: string };

interface IdentityChange {
  now: Identity | null;
  knownAccountId: string | null;
}

interface Account {
  id: string;
  label: string;
  provider: Provider;
  folder: string;
  enabled: boolean;
  sameAsMain: boolean;
  identity: Identity | null;
  identityChange: IdentityChange | null;
  usage: Usage | null;
  usageError: UsageError | null;
  /** The account's limit resets (a Claude account's, read with its usage; a Codex account's banked resets), one per reset, soonest to expire first (never-expiring last); empty while signed in as someone else. Read every 3 hours with a usage reading on its interval (and on its first reading since Warden started; a failed read is asked again on the next one) and whenever usage is refreshed by hand; between reads, and after an answer without them, the last list stays. */
  resets: Reset[];
  hooks: HookSetup;
}

/** One reset the account can use: what using it needs (a Claude grant's id, the same for each reset the grant has left, or a Codex credit's id; null when the answer gave none in the expected form, or in a reading saved before ids were kept: such a reset cannot be used until it is read again), its short name (Claude's by its kind: "5-hour reset", "Weekly reset", "Full reset", "Fable reset"; Codex's the backend's own), and when it expires (ms; null: never). The id is not a secret. */
interface Reset {
  id: string | null;
  title: string | null;
  expiresAt: number | null;
}

/** 'notTrusted': Codex only; the hook is in hooks.json but config.toml does not trust all of it yet. */
type HookSetup = 'installed' | 'notTrusted' | 'notInstalled';

type SignInState = 'signedIn' | 'signedOut' | 'expired' | 'otherAccount';
interface SignIn {
  state: SignInState;
  email: string | null;
}
interface Main extends Account {
  signIn: { state: 'signedIn' | 'signedOut' | 'expired' | 'otherAccount'; email: string | null };
}
type Problem =
  | { kind: 'claudeAppMissing' }
  | { kind: 'claudeAppSignedOut' }
  | { kind: 'cliMissing'; provider: Provider; name: string }
  | { kind: 'cliTooOld'; provider: Provider; name: string; version: string; needed: string };
interface Delegation {
  mainWorksItself: boolean;
  delegated: { accountId: string } | null;
  mainReason: string;
  delegatedReason: string;
  /** Main's hand-off point: the whole percent of its week from which it delegates (100 less the reserve now, rounded down); the same number decides. */
  handOffAt: number;
}

type Theme = 'system' | 'light' | 'dark';

interface Settings {
  reserve: number;
  usageIntervalMin: number;
  theme: Theme;
  /** Desktop app: closing the window leaves Warden in the tray. */
  keepInTray: boolean;
  /** The user's usual work hours, or `null`: every hour counts. */
  workHours: WorkHours | null;
  /** How many days before a limit reset expires the Overview shows it as a warning: whole days, 1 to 14, default 3. */
  resetWarningDays: number;
  /** Whether the Overview's sessions table shows ended and not-known sessions; default `true`. Set only by the checkbox in the Sessions header, not in Settings › General. */
  showQuietSessions: boolean;
}

/** One window on each work day, in local time; no overnight hours. */
interface WorkHours {
  /** ISO weekdays, 1 = Monday to 7 = Sunday, ascending, no repeats, at least one. */
  days: number[];
  /** Minutes after local midnight, in steps of 30, `from < to <= 1440`. */
  from: number;
  to: number;
}

/** A change to the settings. `keepInTray` and `startWithWindows` only in the desktop app. */
type SettingsPatch = Partial<Settings> & { startWithWindows?: boolean };

interface State {
  hooksOff: boolean;
  version: string;
  setupDone: boolean;
  dataDir: string;
  /** The rolling log files' folder, inside the data folder. */
  logDir: string;
  settings: Settings;
  accounts: Account[];
  main: Main | null;
  problems: Problem[];
  lastRead: number | null;
  revision: number;
  homeDir: string | null;
  delegation: Delegation;
  /** The warden.exe the hooks run; null when not found beside the server. */
  routeTool: string | null;
  sessions: Session[];
  activity: { accountId: string; bins: Bin[] }[];
  /** Present only in the desktop app. */
  desktop: { startWithWindows: boolean } | null;
  /** A newer Warden the desktop app found (release builds); null when there is nothing newer and in the demo in the browser. */
  update: Update | null;
}

/** A newer Warden: downloading (`downloaded` the percent done in steps of 5, null before it starts or while its size
 * is not known) or ready to install. The state changes at most once per step, so the window is not woken for every
 * chunk. */
interface Update {
  version: string;
  downloaded: number | null;
  ready: boolean;
}

interface Candidate {
  provider: Provider;
  folder: string;
  label: string;
  identity: Identity | null;
  signedIn: boolean;
  known: boolean;
}

type SessionState = 'working' | 'waiting' | 'idle' | 'done';

interface Session {
  parentId: string | null;
  id: string;
  provider: Provider;
  /** The account whose folder it runs in; it counts for that account unless signedInAs is set. */
  accountId: string;
  /** The email the folder was signed in as instead of its account when the session started; it then counts for no account. */
  signedInAs: string | null;
  state: SessionState;
  doing: string;
  project: string | null;
  name: string | null;
  startedAt: number;
  lastEventAt: number;
  endedAt: number | null;
  subagents: { type: string }[];
  /** list: seen in the CLI's own session list, so its state is not known. */
  source: 'hook' | 'list';
}

/** A 5-minute bin that saw a change; bins without one carry on the `last` count before them. */
interface Bin {
  at: number;
  working: number;
  last: number;
  waited: boolean;
}
```

## Endpoints

| Method and path                        | Body                                                          | Answer                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| -------------------------------------- | ------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `GET /api/state`                       |                                                               | `State`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `GET /api/state/wait?since=<revision>` |                                                               | `State` as soon as its `revision` differs from `since`, or after 50 s either way. The open window keeps one of these waiting, so it updates the moment something changes without polling.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `POST /api/usage/refresh`              |                                                               | `State`, after reading every Claude account now (its usage and, unlike a reading on the interval, always its limit resets) and each Codex account's banked resets (Codex usage comes from the logs as they change), and the CLIs' session lists for accounts no status hook has reported                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `PUT /api/settings`                    | `Partial<Settings> & { startWithWindows?: boolean }`          | `State`. `400` for another interval; `keepInTray` and `startWithWindows` only in the desktop app (`400` in the demo in the browser); `workHours` an object as `WorkHours` describes (`400` "Work hours need at least one day, and an end after their start, in steps of 30 minutes." otherwise) or `null` to clear it; `resetWarningDays` a whole number from 1 to 14 (`400` "Warn from 1 to 14 days before a reset expires." otherwise); `showQuietSessions` a boolean (`400` otherwise). Nothing changes when any part is refused.                                                                                                                                                                                                                                                               |
| `POST /api/tools/scan`                 |                                                               | `State`, with missing or old CLI problems checked again                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `GET /api/setup/candidates`            |                                                               | `Candidate[]`: the folders `CLAUDE_CONFIG_DIR` and `CODEX_HOME` name, then account folders found in `~/.warden` (`claude-*`, `codex-*`), then `~/.claude-*` and `~/.codex-*` in the home; never `~/.claude`, `~/.codex` or `~/.warden` itself                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `POST /api/setup`                      | `{ accounts: { provider, folder, label }[]; skip?: boolean }` | `State`; main is implied. Start installs main's hooks even with an empty list; `409` while the Claude app is missing or signed out. `skip: true` requires an empty list, finishes setup without installing hooks, and works while the app is missing or signed out. Adding the first Warden account after Skip installs main's hooks.                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `POST /api/setup/restart`              |                                                               | `State` with `setupDone: false`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `POST /api/accounts`                   | `{ provider, folder, label }`                                 | `State`; adds a signed-in Warden folder. `409` for main or duplicates; `400` for the Codex app, Warden’s own sign-in folder or `~/.warden` itself.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `PATCH /api/main`                      | `{ label }`                                                   | `State`; renames fixed main.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `POST /api/accounts/{id}/sign-in`      | none                                                          | `State`; opens the official sign-in for that Warden account, including Undo after an identity change; usage is read again as soon as the sign-in's CLI exits.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `POST /api/accounts/{id}/hooks/repair` | none                                                          | `State`; repairs the wanted hooks, with Codex trust; id `main` repairs both main hooks.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `POST /api/accounts/{id}/resets/use`   | `{ resetId }`                                                 | `State`, once the reset is used (see Using a limit reset): it leaves the account's `resets` at once, and the account's usage and limit resets are read again, as a refresh by hand reads them. Only for a reset on the account's list now (`409` "This reset is no longer on the account's list." otherwise) and not expired by now (`409` "This reset has expired."), only while its folder is signed in as the account itself, with an identity read now (`409`; a login without one is signed out and nothing is sent), never for the Codex app's folder (`400`). The provider's refusal or a failed request is `409` with the reason in plain words ("This reset was already used.", "Nothing to reset: the account is not at a limit.", "Try again later.", and so on); the reset then stays. |
| `POST /api/main/sign-in`               | none                                                          | `State`; starts the official Claude sign-in in Warden's own `main-sign-in` folder, and reads usage again as soon as the sign-in's CLI exits. Refused renewal stays expired until this explicit sign-in; the old login is not retried while the sign-in window is pending.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `POST /api/accounts/new`               | `{ provider, label }`                                         | `State`; creates `~/.warden/claude-<name>` or `~/.warden/codex-<name>` (and `~/.warden` when missing) and starts the official sign-in (`claude auth login` or `codex login`) in a new window. `400` when that CLI is not on PATH; when `~/.warden` is (through a link) the Codex app's folder or Warden's own sign-in folder, `400`, or main's, `409`, and nothing is made                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `PATCH /api/accounts/{id}`             | `{ label?, enabled? }`                                        | `State`; main uses its own Rename endpoint. The switch installs/removes the status hook.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `POST /api/accounts/{id}/identity`     | `{ action: 'accept' }`                                        | `State`; accepts the new identity as this account's from now on                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `DELETE /api/accounts/{id}`            |                                                               | `State`; Warden forgets the account. Its hooks are removed (as below); the folder is otherwise left as it is. `500` when the hooks could not be removed; the account then stays.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `POST /api/update/install`             |                                                               | The desktop app installs the downloaded update and starts again: it closes for the installer inside the request, so a successful one gets no answer. `400` in the demo in the browser; `409` when no update is ready; `500` when the installer could not start (the update stays ready).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `POST /api/logs/open`                  |                                                               | `State`, after the desktop app opened `logDir` in Explorer (`500` when it could not); `400` in the demo in the browser, which shows the path only.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |

A Warden account has only its status hook, exactly while `enabled` is true. Main always has both hooks once setup with accounts finishes. Switching off or removing an account restores its backup; a missing or untrusted wanted hook offers Repair.

## Using a limit reset

Only when the user confirms using a reset in the app (`POST /api/accounts/{id}/resets/use`); Warden never uses a reset by itself. Each use sends one request with a new random request id (a UUID), the login read for it and dropped with it, and follows no redirect; only a successful answer's body is read, capped at 64 KB, and an error's body never. `401`, `403`, `429` and `5xx` answer short reasons ("HTTP 429: rate limited"); the log says `{label}: reset used ({short name})` or `{label}: reset refused ({reason})`, never a token or id.

- **Claude** (main with Warden's own sign-in, checked and renewed as for a usage reading; a Warden account with the login in its own folder, never renewed: an expired one is not sent), as the Claude CLI sends it: `POST https://api.anthropic.com/api/organizations/{organizationUuid}/reset_rate_limits` with the usage request's headers (`Authorization: Bearer`, `anthropic-beta`, the CLI's `User-Agent`, `x-app: cli`) and `Content-Type: application/json`, body `{"program": "cedar_ember", "grant_id": "<grant id>", "request_id": "<request id>"}`. The organization is the one the sign-in's `oauthAccount.organizationUuid` names (Warden's own sign-in's for main). The grant id must match `^[a-z0-9_-]{1,40}$`, the request id `^[A-Za-z0-9_-]{1,64}$` and the organization's letters, digits and `-` (1 to 64); otherwise nothing is sent. Only `result: "reset"` is a success; `already_used`, `not_limited`, `cooldown`, `ineligible` and `unavailable` are said in plain words, and nothing else from the answer is kept.
- **Codex**, as Codex's own client redeems a banked reset (`openai/codex`, `codex-rs/backend-client`, `consume_rate_limit_reset_credit_by_id`), with the access token and account id from the account's `auth.json`: `POST https://chatgpt.com/backend-api/wham/rate-limit-reset-credits/consume` with the list request's headers (`Authorization: Bearer`, `chatgpt-account-id`, `User-Agent: codex-cli`) and `Content-Type: application/json`, body `{"redeem_request_id": "<request id>", "credit_id": "<credit id>"}`. The credit id must be letters, digits, `_`, `-`, `.` or `:` (1 to 128). Only `code: "reset"` is a success; `nothing_to_reset`, `no_credit` and `already_redeemed` are said in plain words.

## The status hook

Each account's status hook runs `warden.exe` once per event, with the event's JSON on its standard input:

```sh
<warden.exe> event --account <id> --data-dir <data folder>
```

`warden event` keeps only the hook event's name, the session id, the working folder, the transcript path (a path; the file is never read or sent), tool names, subagent ids and types, the notification kind, and `parent_session_id` from `WARDEN_PARENT_SESSION` (`warden_core::pipe::HookEvent`). The parent id allows only letters, digits, `-` and `_`, at most 128 characters; anything else is dropped. The prompt, tool inputs and outputs, and messages are skipped as the input is read and never leave the hook. It sends that to the core over a named pipe only the current Windows user can open (a security descriptor allowing that user alone; the first instance created exclusively, so nobody can take the name first), under a fresh name each start, written with the core's process id to `pipe.json` in the data folder. `warden event` connects only when the process at the other end is the one `pipe.json` names, waits for the core's one-byte answer, prints nothing and always exits 0: within a second however the core behaves, at once when Warden is not running. The core refuses a message with any other field. A Claude transcript inside the account's folder (no link on the way to it) is read only for its newest `custom-title` record, or without one its newest `ai-title` (Claude Code's own generated title; a custom title always wins, whichever is newer), keeping only the title, from where it was last read, on its session's events and when it changes on disk. Sessions and activity live in memory only.

The core finds each session's CLI process from the pipe: Windows names the process that connected (the hook's `warden.exe`), and its parent chain leads to `claude.exe` (or `node.exe` for an npm install) or `codex.exe`. Warden waits on that process (no polling): when it exits, its sessions end, two seconds later so late events arrive first. A Codex session becomes done; a Claude session that did not report its end (crashed or killed) goes at once. Only sessions whose process was not found are dropped after 24 hours without an event. A session seen only in a CLI's own session list (`source: 'list'`) is added if it was written to in the last 30 minutes and dropped once its last write is older than that (read for every account when Warden starts, before the hooks' pipe opens and before the first usage reading, so the first answer already has these sessions, each under its main session where a saved parent link names one; afterwards only on a refresh by hand, for accounts no status hook has reported). A main session that a shown delegated run's parent link names (saved, or in its event) but that is not in those lists is looked up by its id in the accounts' own session files, whatever its age (main's folder first; at start, and when a run the hook reports names it, once per id per start; only plain files and folders, no link, junction or other reparse point followed anywhere below the account folder, `projects` and `sessions` included, which the 30-minute lists keep to as well), and stays while one of its runs is shown, past the 30 minutes; once none is, the usual rule drops it, and it comes back with the run's next event from what was found, without another lookup (the last 400 found are remembered). A hook event for it takes it over, and the rules above apply from then on.

It is installed like the routing hook, in the same files and with the same backup: one group per event, after the user's own groups, each with one handler running that command with `"async": true` (in the background, so the turn never waits for it) and a timeout of 5 s.

- Claude events: `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PermissionRequest`, `Stop`, `SubagentStart`, `SubagentStop`, `SessionEnd`.
- Codex events: the same less `SessionEnd`, which Codex does not have; `config.toml` gets a trust entry per handler, as for the routing hook (`<folder>hooks.json:<event in snake_case>:<group>:0`).

Installing or removing either hook writes both in one go, so the groups' places named in Codex's trust keys stay right. When that moves one of the user's own Codex hooks (Warden's group taken out from before it), the hook's existing trust entry moves to its new key with it, only for the same unchanged hook (its trusted hash does not depend on the place, so nothing new is trusted). With neither hook on, the files are put back byte for byte (or only Warden's entries are taken out, when they were changed since). Each file is read again just before Warden replaces it, so a change Claude, Codex or an editor made in between is kept (except one landing in the moment between that reading and the replacement: the CLIs take no lock Warden could wait for).

When Warden starts, it repairs its own outdated hook entries and logs the repair. Missing or untrusted wanted hooks stay visible on the account and can be repaired through its endpoint. There is no update note.

## Saved files

In the data folder: `--data-dir`, or `%APPDATA%\Warden`, shared by the server, the desktop app and `warden route`.

- `main-sign-in/`: Warden's own Claude sign-in. Only its `.credentials.json` may be renewed, by the holder of `server.lock`, and saved before use; each renewal is logged with when it holds until ("renewed Warden's sign-in to main; it holds until Fri 15:20"), and a refused one once ("Warden's sign-in to main was refused on renewal; sign in again"), never the login. Account login files are never renewed or written. No tokens are saved in any other Warden file.
- `settings.json`: version 2, app settings, `reserve` (0–50 in steps of 5, default 25), `workHours` (optional; a file without it, or with a value of the wrong shape or out of bounds, means every hour counts), `resetWarningDays` (1–14, default 3; missing, of the wrong type or out of range loads as 3), `showQuietSessions` (default `true`; missing or of the wrong type loads as `true`), the small `main` record (label and remembered identity), and Warden accounts. Main is always `~/.claude`, id `main`, outside the account list. A Codex login remains separate even when its email matches main; `sameAsMain` compares Claude accounts. Older or unsupported future settings are set aside as `settings.json.bad` (then `.bad.1`, `.bad.2`, and so on, preserving earlier files) and setup starts again. Account ids use an `idGeneration` namespace after a set-aside and are never reused. Version 1 archives supply only the old folder list for one cleanup attempt; `settings-cleanup.json` records the attempted archives. The Codex app's folder is always excluded. Routing settings are no longer saved.
- `readings.json`: the last usage reading per account, its error, the identity-change flag, the account's limit resets (id, short name and expiry only), and `checkedAt` (when usage was last read for every account), for `warden route`. No tokens.
- `route-sessions/<session>`: one small file per session with what it was last told (a key, not the text), so `warden route` stays silent until something changes. At most 200 sessions, forgotten after 7 days.
- `hooks/<account id>.json`: the backup of each config file the hooks were written into, as it was before, so removal puts it back byte for byte.
- `logs/warden-<YYYY-MM-DD>.log`: the server's and the desktop app's log, one file a day in local time (a new one at local midnight; each line starts with the local time and its UTC offset, `2026-11-10 10:15:03 +01:00`), the last 5 days kept, at most 512 KB a day and 2 MB in all (a day's further lines are dropped; the oldest files go first). Written only by the running server; never a token, prompt, tool input or message.
- `window.json` (desktop app): the window's place, in physical pixels: `x`, `y` (its outer position), `width`, `height` (its inner size, the one from before maximizing while maximized), `maximized`, and `maximizedAt` (where the maximized window was, so it opens maximized on that screen). Written when the window closes, when Warden quits and before an update installs; a place no longer on any screen is not used, and the window opens at its default size, centred.
- `update/` (the installed desktop app): `installer`, a downloaded update waiting to be installed (removed at the next start), and `open-window`, left by Install and restart so the restarted Warden opens its window.
- `session-parents.json`: which main session started each delegated run (provider, the run's session id, its main session's id, when learned), so the sessions tree is whole again after Warden restarts. Session ids only; at most 200, each kept 24 hours.
- `refresh.lock`: present while a background refresh runs.
- `pipe.json`: while the core runs, the name of its pipe and its process id, for `warden event`.
- `server.lock`: held open (unshared) by the running server, and by `warden refresh` while it runs. Only one server runs per data folder, and only one writer touches `readings.json` at a time: a refresh finds the server running and leaves usage to it; a server starting during a refresh waits for it (up to 30 s).

## The routing hook

Main works itself while it has used less of its week than its hand-off point, 100 less its reserve now, rounded down to a whole percent (`handOffAt`; the same number decides). The reserve shrinks with what is left of main's week before its weekly reset: the time left (a reset more than 7 days away counts as 7 days), or, with `workHours` set, the work hours left (each work day's window in local time, with Windows' offset at that moment) out of the work hours in a week. With no weekly reset known, main keeps its full reserve. Delegated work skips accounts at 100% or more of their week, as well as those switched off or at 90% or more of their 5-hour window. A weekly reset that has passed counts as 0%, including for an expired login; an account with no reading yet is eligible. When nothing can take delegated work, main keeps it and the advice says so once per decision change.

One group of two command handlers on `UserPromptSubmit` (every message), in the account's `settings.json` (Claude) or `hooks.json` (Codex), added after the user's own groups:

- `<warden.exe> route --account <id> --data-dir <data folder>` (timeout 10 s): prints the guidance when it changed for this session (read from `session_id` on stdin), otherwise nothing; nothing at all while the account's folder is signed in as someone else, or for an account Warden no longer has. Never fails the message.
- `<warden.exe> refresh --if-stale --account <id> --data-dir <data folder>` (async, timeout 60 s): when the readings are older than the usage interval and a minute, reads every account's usage once (never asking for limit resets: routing does not use them) and merges it into the readings as they are then.

How the CLIs run Warden's handlers (all three hooks):

- Claude Code: the exec form, `{ "type": "command", "command": "<warden.exe>", "args": ["route", "--account", "<id>", "--data-dir", "<data folder>"], ... }`: the program is started directly with those arguments and no shell, so paths need no quoting. It needs Claude Code 2.1.139 or later; with an older Claude CLI found (its version read from the files beside it), Warden writes no Claude hooks and a `cliTooOld` problem says so.
- Codex, which has no argument-list form: one line run by `cmd.exe /C`, paths with forward slashes, unquoted when they have nothing but letters, digits and `_.:/~-`, otherwise in double quotes. `cmd.exe` expands `%NAME%` even inside quotes, so Codex's hooks are refused (`500`) when `warden.exe` or the data folder has a `%` in its path.

Which shell Claude Code runs the AI's commands in is decided as Claude Code decides it, from files only (nothing is started), for the routing guidance's commands: Git Bash when `CLAUDE_CODE_GIT_BASH_PATH` (the account's `settings.json` `env`, then Warden's environment) names an existing `bash.exe`, `sh.exe`, `bash` or `sh`, when `Git\bin\bash.exe` is in `Program Files` or `Program Files (x86)` on the C: drive (Claude Code looks there whatever `%ProgramFiles%` says), or when `bin\bash.exe` sits beside the folder of the first `git` on PATH; PowerShell otherwise.

The routing advice runs only in main and stays silent while the decision and commands are unchanged: it names Warden's pick, then lists every switched-on Warden account, one line each, with its command and, for one Warden skips, why. Accounts skipped for their limits keep their command (the user may still choose them); one signed out, signed in as someone else, or on main's own Claude account gets no command. It says the user can always choose: work in main, or run on the account they name. Whenever it lists a command, its second line says to run a delegated command in the background and read its result when it finishes, since Claude Code stops waiting for a long command in the foreground, and its third that anything the session starts that runs Claude or Codex for it (a review script) is delegated work too, run with that account's variables, so it never falls back to a CLI's default folder, and its fourth what to put in the placeholders: `<name>`, a few words saying what the run does, and `<task>`, the task (only `<task>` when no Claude command is listed, as Codex's command has no name). It holds no percentages, times or ages, so it changes only with the pick, the mode, a skip, the accounts or a command. For example, below the hand-off point, in Git Bash:

```text
Warden: work in this session. If the user asks to delegate, Warden's pick is Second. The user can always choose: work here, or run on whichever account they name.
Run a delegated command in the background and read its result when it finishes; a long run in the foreground is cut off.
Anything you start that runs Claude or Codex for you (a review script, say) is delegated work too: run it with the same variables as that account's command above.
Replace `<name>` with a few words saying what the run does, and `<task>` with the task.
- Second (Warden's pick): WARDEN_PARENT_SESSION='<id>' CLAUDE_CONFIG_DIR='<folder>' claude -p --permission-mode auto --name "<name>" "<task>"
- Codex work (skipped: near its 5-hour limit): WARDEN_PARENT_SESSION='<id>' CODEX_HOME='<folder>' codex exec --approve-for-me "<task>"
- Spare (skipped: signed out)
```

Past the hand-off point the first line is "Warden: main is handing work off now, so run each task on <account> (Warden's pick); here, only talk to the user and check results. Whenever the user asks, work in this session instead, or run on the account they name."; with nothing to delegate to, "Warden: no other account can take delegated work now, so work in this session. If the user names an account, run it there." Parent ids come only from hook stdin: ASCII letters, digits, `-` and `_`, at most 128 characters; a missing or invalid id omits the variable.

The guidance names the exact command for each account it sends work to, written for the shell the session's AI runs commands in, with the account's folder in single quotes and the CLI as found on PATH:

- Claude Code with Git Bash: `WARDEN_PARENT_SESSION='<id>' CLAUDE_CONFIG_DIR='<folder>' claude -p --permission-mode auto --name "<name>" "<task>"` (a `'` in a path written `'\''`).
- Claude Code in PowerShell, decided as above (`warden route` reads it for the session's account on every message; the preview with the hook setups), and Codex sessions (always PowerShell): `& { $was = $env:CLAUDE_CONFIG_DIR; $parentWas = $env:WARDEN_PARENT_SESSION; try { $env:CLAUDE_CONFIG_DIR = '<folder>'; $env:WARDEN_PARENT_SESSION = '<id>'; claude -p --permission-mode auto --name "<name>" "<task>" } finally { $env:CLAUDE_CONFIG_DIR = $was; $env:WARDEN_PARENT_SESSION = $parentWas; } }`, so the variable is put back for the session's later commands. A quote in a path, straight or typographic, is doubled.

The examples show a Claude account; a Codex account is the same with `CODEX_HOME` and `codex exec --approve-for-me "<task>"`. Both run in each CLI's own unattended mode (`--permission-mode auto`, `--approve-for-me`), so a delegated run can edit inside the project within the limits that mode keeps, with nobody there to approve each step.

The shell is part of what a session was last told, so a session is told again when it changes.

For Codex, `config.toml` gets one `[hooks.state.'<folder>\hooks.json:user_prompt_submit:<group>:<handler>']` entry per handler (both path spellings) with `enabled = true` and `trusted_hash`, as Orca writes them.

## Uninstall

`warden unhook --data-dir <folder>` removes Warden's hooks from fixed main and every configured Warden account, restoring their backups. It reads no login or usage files and makes no network calls. It can run while the server holds its lock: the server reports the missing hooks and does not install them again. Version 1 folder lists are also used for cleanup. Main is always attempted, even if settings cannot be read or parsed; valid folders are still cleaned when another entry or hook fails. Unknown settings formats are preserved, and any cleanup failure returns a nonzero exit status.

The installer runs this command on `%APPDATA%\Warden` before removing its files. If cleanup fails, it shows one warning and continues; silent uninstall defaults to OK. An installer update keeps the hooks for the replacement app's startup repair.
