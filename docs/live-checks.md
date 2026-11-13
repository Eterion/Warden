# Live checks

What the tests cannot show: Warden against your real Claude and Codex accounts and the real CLIs. Each check needs you there. Try the demo first (`npm run demo`, made-up accounts, nothing real is read), then go through these in order.

The checks run in a development build of the desktop app, with a data folder of their own, so nothing here mixes with a later everyday setup. The browser (the development server, `npm run dev`, the demo) is for made-up accounts only; never point it at your real ones.

Every check except the hook checks also runs with `--no-hooks`: your real home, real accounts and real readings, with nothing written into any account folder. The window says "Hooks are off for this run". `--no-hooks` and `--home` exist only in development builds, never in the installed app.

```powershell
npm run build -w ui
cargo build -p warden-cli
cargo run -p warden-desktop -- --data-dir $env:TEMP\warden-live --no-hooks
```

(`warden-cli` builds the `warden.exe` the hooks run.)

## 1. Setup finds main and your accounts (read only)

- **Do:** open Warden on the empty data folder; first-run setup shows main and the folders it found.
- **Expect:** main is the Claude app's account, with the right email. Every account folder you use, in `~/.warden` (`claude-*`, `codex-*`) or as `~/.claude-*` and `~/.codex-*`, is listed, ticked, with its email and a suggested name. `~/.claude`, `~/.codex` and `~/.warden` itself are never listed. The list is visible at the window's size, or the setup shows that it scrolls (a divider above the buttons while more is below). Do not press Start yet.
- **Restore:** nothing was written.

## 2. Warden's own sign-in to main, and its first reading

- **Do:** in setup, Sign in under Main's usage, and sign in as main's email in the official sign-in that opens. Then press Start. This is the first time a real login is used, only to call the usage endpoint; give your go-ahead first.
- **Expect:** the sign-in runs `claude auth login` in a window of its own and opens the browser. As soon as that window closes, main's first reading appears, with no Refresh and no wait for the usage interval, and setup says Warden is signed in as main's email. Main's 5-hour and weekly use and reset times match what `/usage` shows in a main session (give or take what changed in between), and so do the other Claude accounts'. Signing a Warden Claude account in (Sign in on its row) shows its fresh reading the same way.
- **Restore:** nothing; Warden's sign-in lives in its own data folder.

## 3. The first real renewal

- **Do:** leave Warden running past the access token's expiry (about 8 hours).
- **Expect:** main's reading stays current without a new sign-in, and the Claude app stays signed in. The log (Open log folder) shows "renewed Warden's sign-in to main; it holds until …" at the renewal, and no login.
- **Restore:** nothing.

## 4. An expired Warden account

- **Do:** leave Warden running while one of its Claude accounts sits unused until its login expires (or wait for one that already has).
- **Expect:** that account keeps its last reading: its bars turn grey, with the reading's age and a tooltip saying its login is idle, and no amber. After Claude next renews the login (use the account once), the next reading is current again, in colour.
- **Restore:** nothing.

## 5. Codex usage updates as it is used

- **Do:** run a short `codex exec` task in one of Warden's Codex accounts.
- **Expect:** within seconds of the task's first answer, that account's reading moves and shows "read just now"; no refresh is needed.
- **Restore:** nothing.

## 6. Identity check (on a copy)

- **Do:** copy one of Warden's Codex folders to a test folder (for example `~/.codex-idcheck`), add the copy as an account (Add account, An existing folder), then sign the copy in as your other Codex account in a new PowerShell window (`$env:CODEX_HOME = "<the copy>"; codex login`).
- **Expect:** the copy's row in Settings, Accounts shows "Signed in as <the other email>, which is <the other account's name>" in amber, with Accept and Undo beside it, and its usage is kept apart. Accept makes the new identity the copy's.
- **Restore:** close that PowerShell window (it is the only one with `CODEX_HOME` set to the copy), remove the account in Settings, then delete the copied folder.

## 7. A new account in a test folder

- **Do:** Settings, Accounts, Add account, A new account, with a test name (for example "Live test").
- **Expect:** `~/.warden/claude-live-test` (or `~/.warden/codex-live-test`) is created and the official sign-in opens in a new window. Close it without signing in.
- **Restore:** remove the account in Settings and delete the folder.

## 8. Delegation

- **Do:** set My work hours in Settings, Accounts. Compare the status bar's one item ("Work goes to Main until N%", or the account the work goes to) with the readings; hover it for its reason. Then drag "Keep for my own work", and change the work hours.
- **Expect:** main works itself while it has used less of its week than its hand-off point: 100 less the reserve now, rounded down, the reserve shrinking with the work hours left before main's weekly reset (evenings and weekends do not shrink it). Past it, the work goes to the account with the most allowance left in its week for the time until its reset, skipping accounts switched off, at 90% or more of their 5-hour limit, or with their week used up. The status bar's "until N%", the mark on main's weekly bar on the Overview and "Hands off at N% now" under the slider agree; dragging the reserve or changing the work hours moves all three.
- **Restore:** set the reserve back.

## 9. Idle memory

- **Do:** with every real account added, switch on Keep running in the tray (Settings, General) and close the window. Leave Warden idle for a minute, then look at `warden-desktop.exe` in Task Manager (Details, Memory (private working set)).
- **Expect:** within the tray memory budget (`desktopTrayMemoryMB` in `checks/budgets.json`).
- **Restore:** nothing.

## 10. One page load, said once

- **Do:** open Warden's window, close it to the tray, and open it again; then look at the log (Open log folder).
- **Expect:** "the window loaded its page" once for each opening, each followed once by its styles and its first API answer.
- **Restore:** nothing.

## Hook checks, on a test home

The routing hook, Codex's hooks, the status hook, the sessions tree, a switch on and off, Repair and `warden unhook` run on a test home instead of your real one:

```powershell
cargo run -p warden-desktop -- --data-dir $env:TEMP\warden-hooks --home $env:TEMP\warden-home
```

The test home holds a copy of a `.claude` as main, and a test account signed in from a terminal (for example `.claude-test`, or `.codex-test`). `--home` points only Warden there: every CLI you start yourself needs its folder set in its own PowerShell window first, or it runs on your real accounts. Close each such window when done (they are the only ones with these set):

```powershell
# Main (a Claude session in the test home's main):
$env:CLAUDE_CONFIG_DIR = "$env:TEMP\warden-home\.claude"
# The Claude test account (sign it in with `claude auth login` the first time):
$env:CLAUDE_CONFIG_DIR = "$env:TEMP\warden-home\.claude-test"
# The Codex test account (sign it in with `codex login` the first time):
$env:CODEX_HOME = "$env:TEMP\warden-home\.codex-test"
```

A delegated run started from main's advice sets its own account folder.

- **Routing hook:** start a main session in the test home and send a few messages. Claude runs the hook without delaying the message; Warden's advice appears at the session's first message, not again while nothing changes, and once more after something that moves work (the reserve, or a switch). The advice names Warden's pick and lists every switched-on account with its command, the skipped ones with why (no command for one signed out or signed in as someone else), and says you can always choose: ask the session to work in main, or to run on an account you name, and it does. With a command listed, it also says to run delegated commands in the background, and the session does: a long delegated run is not cut off, and the session reads its result when it finishes.
- **Codex hooks:** switch the Codex test account on and run `codex exec` in a project Codex has never trusted. The status hook runs with the user-level trust Warden writes in `config.toml`, with no project trust entry; the trust entry's path spelling matches the home folder as Codex writes it; and Codex runs the hook in `cmd.exe`, the shell Warden quotes it for.
- **Status hook and the sessions tree:** work in main and start a delegated run from its advice. Sessions appear on the Overview as they start, with working, waiting for you, idle and subagents shown as they happen; the delegated session sits under the main session that started it. A Claude session reaches done when it ends (`SessionEnd`; Codex has no such event, so its sessions stay idle). Neither CLI's turns feel slower with the hook on.
- **A switch on and off, and Repair:** switching an account off removes its status hook and puts its config file back byte for byte; switching it on installs it again. Remove the hook by hand: the account shows the problem in amber with Repair, which puts it back.
- **`warden unhook`:** run it as the uninstaller does, pointed at the test home (`--home`, development builds only), so it never touches your real main folder:

  ```powershell
  target\debug\warden.exe unhook --data-dir $env:TEMP\warden-hooks --home $env:TEMP\warden-home
  ```

  It removes Warden's hooks from main and every account in the test home, restoring their backups.

**Restore:** delete the test home and the data folder.

## Switch-over

Only once everything above passes: see brief 4.10.
