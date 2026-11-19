// Accessibility check (axe-core, WCAG 2.2 A and AA, including contrast) on the rendered pages, in light and dark,
// with a screenshot of each for the UX reviewer in artifacts/screenshots/.
//
// Builds the UI, serves it on 127.0.0.1 with `vite preview` as the demo server does (its token traded for a
// cookie once, then refused without it, and its Content-Security-Policy), and drives the Microsoft Edge that ships with Windows
// through playwright-core, so no browser is downloaded. The local server's API is stood in for by made-up states
// (checks/fixtures/ui-state.ts), so no real account is read. `?desktop` pages run where the desktop window runs them
// (at http://warden.localhost with its API there too, under the window's policy), passed on to the same preview server. Any
// request the policy blocks fails the check.

import { mkdirSync, readFileSync } from 'node:fs';
import type { ServerResponse } from 'node:http';
import { createRequire } from 'node:module';
import { join } from 'node:path';
import { chromium } from 'playwright-core';
import { build, preview, type Plugin } from 'vite';
import {
  candidates,
  desktopState,
  fiveCandidates,
  healthyState,
  hooksState,
  normalState,
  problemsState,
  quietHiddenState,
  resetsState,
  setupState,
  treeState,
} from './fixtures/ui-state.ts';
import { browserCsp, desktopCsp } from './lib/csp.ts';
import { ROOT, changedFiles } from './lib/repo.ts';
import { affectedPages } from './lib/pages.ts';

type Fixture =
  | 'normal'
  | 'desktopDownloading'
  | 'desktopUpdate'
  | 'hooksOff'
  | 'setup'
  | 'offline'
  | 'desktop'
  | 'mainSignedOut'
  | 'mainExpired'
  | 'mainOtherAccount'
  | 'mainHooksMissing'
  | 'accountHooksMissing'
  | 'setupSignedOut'
  | 'setupOtherAccount'
  | 'problems'
  | 'tree'
  | 'quietHidden'
  | 'healthy'
  | 'resets'
  | 'resetsRefused'
  | 'resetsUsed'
  | 'hooks'
  | 'workHours'
  | 'delegated'
  | 'noTarget'
  | 'mainUnknown'
  | 'setupFive';

/** What to do on a page once it is ready, to reach a state such as a menu's modal (Playwright selectors). */
/** `hover` rests the pointer on an element; `press` also holds the button down on it, until the page is checked. */
type Step =
  | { click: string }
  | { focus: string }
  | { hover: string }
  | { press: string }
  /** Presses a key (`Enter`, `ArrowDown`) on whatever has focus. */
  | { key: string }
  /** Fails unless the element with focus matches this selector (waiting up to 10 s for it to). */
  | { focused: string }
  | { fill: string; text: string }
  | { waitFor: string }
  /** Waits until no element matches (a row that has left, say). */
  | { waitGone: string }
  /** Pauses every running animation this far (ms) into its timeline: a still of one moment in it. */
  | { freeze: number }
  /** The page's next `Math.random()` answers this, then it is random again (to pick what a random choice shows). */
  | { random: number }
  /**
   * Fails unless the element's `::before` is masked with `drawing`'s SVG (shown through a mask, kept by the CSS build)
   * and that SVG loads and decodes under the page's policy.
   */
  | { masked: string; drawing: string }
  /** Fails the state unless the element is wholly visible inside the scrolling area named by `within`. */
  | { inView: string; within: string };

/**
 * The pages and page states to check, each with the made-up state the API answers, and the steps that reach the
 * state from the page. Later steps add theirs here. `?desktop` pages run as inside the desktop app's window (its
 * Tauri bridge stood in for), with the window controls.
 */
export const PAGES: {
  name: string;
  path: string;
  fixture: Fixture;
  ready: string;
  steps?: Step[];
  /** A window size other than the usual 1280 × 800 (first-run setup at the window's smallest). */
  viewport?: { width: number; height: number };
}[] = [
  {
    name: 'settings-main-signed-out',
    path: '/#/settings/accounts',
    fixture: 'mainSignedOut',
    ready: 'button:has-text("Sign in")',
  },
  {
    name: 'settings-main-expired',
    path: '/#/settings/accounts',
    fixture: 'mainExpired',
    ready: 'button:has-text("Sign in again")',
  },
  {
    name: 'settings-main-other-account',
    path: '/#/settings/accounts',
    fixture: 'mainOtherAccount',
    ready: 'button:has-text("Sign in again")',
  },
  {
    name: 'settings-main-hooks-missing',
    path: '/#/settings/accounts',
    fixture: 'mainHooksMissing',
    ready: '[data-hooks] >> text=Missing',
  },
  {
    name: 'settings-account-hooks-missing',
    path: '/#/settings/accounts',
    fixture: 'accountHooksMissing',
    ready: '[data-hooks] >> text=Missing',
  },
  {
    name: 'settings-accounts-hooks-off',
    path: '/#/settings/accounts',
    fixture: 'hooksOff',
    ready: '[role=dialog] >> text=Hooks are off for this run',
  },
  {
    name: 'settings-accounts-problems',
    path: '/#/settings/accounts',
    fixture: 'problems',
    ready: 'button:has-text("Scan again")',
  },
  {
    name: 'first-run-setup-app-signed-out',
    path: '/',
    fixture: 'setupSignedOut',
    ready: 'button[aria-disabled=true]:has-text("Start")',
    steps: [{ focus: 'button:has-text("Start")' }, { waitFor: '[role=tooltip]' }],
  },
  {
    name: 'first-run-setup-main-other-account',
    path: '/',
    fixture: 'setupOtherAccount',
    ready: 'button:has-text("Sign in again")',
  },
  {
    name: 'overview',
    path: '/',
    fixture: 'normal',
    ready: 'main [role=table]',
  },
  {
    name: 'overview-work-delegated',
    path: '/',
    fixture: 'delegated',
    ready: '[data-work]',
  },
  {
    name: 'overview-no-account-can-take-work',
    path: '/',
    fixture: 'noTarget',
    ready: '[data-work]',
  },
  {
    name: 'overview-main-usage-unknown',
    path: '/',
    fixture: 'mainUnknown',
    ready: '[data-work]',
  },
  {
    name: 'overview-login-idle',
    path: '/',
    fixture: 'normal',
    ready: 'main .bg-bar-idle',
  },
  {
    name: 'overview-sessions-tree',
    path: '/',
    fixture: 'tree',
    ready: '[aria-labelledby=sessions-heading] >> text=Started by',
  },
  {
    // The window at its smallest: the sessions table runs below the panel's bottom edge, its screen-reader-only text
    // with it, and the page itself still does not scroll.
    name: 'overview-sessions-tree-small',
    path: '/',
    fixture: 'tree',
    ready: '[aria-labelledby=sessions-heading] >> text=Started by',
    viewport: { width: 800, height: 560 },
  },
  {
    // A browser narrower than the desktop window can be: the sidebar stays beside the panel, nothing wraps below.
    name: 'overview-narrow-browser',
    path: '/',
    fixture: 'tree',
    ready: '[aria-labelledby=sessions-heading] >> text=Started by',
    viewport: { width: 640, height: 560 },
  },
  {
    name: 'overview-sessions-tree-filtered',
    path: '/',
    fixture: 'tree',
    ready: '[aria-labelledby=sessions-heading] >> text=Started by',
    steps: [{ click: 'button[role=checkbox][aria-label$="of Second"]' }, { waitFor: 'text=Show all' }],
  },
  {
    // Codex checked: its waiting session sits under the main session that started it, which shows faded.
    name: 'overview-sessions-tree-faded',
    path: '/',
    fixture: 'tree',
    ready: '[aria-labelledby=sessions-heading] >> text=Started by',
    steps: [{ click: 'button[role=checkbox][aria-label$="of Codex"]' }, { waitFor: 'text=Show all' }],
  },
  {
    // Ended and not-known sessions hidden (the checkbox off): Codex's ended session stays as the faded parent of a shown
    // session on Second.
    name: 'overview-sessions-quiet-hidden',
    path: '/',
    fixture: 'quietHidden',
    ready: '[data-quiet-toggle] [role=checkbox][aria-checked=false]',
  },
  {
    name: 'overview-resets',
    path: '/',
    fixture: 'resets',
    // Main's Claude resets and Codex's banked resets, one of main's expiring inside the warning days (amber).
    ready: '[aria-labelledby=resets-heading] [data-expires-soon]',
    // Wide enough for the resets to stand beside the accounts and sessions.
    viewport: { width: 1600, height: 900 },
  },
  {
    name: 'overview-resets-narrow',
    path: '/',
    fixture: 'resets',
    ready: '[aria-labelledby=resets-heading] [data-expires-soon]',
  },
  {
    // Use on main's full reset: the confirmation, before anything is sent.
    name: 'overview-use-reset',
    path: '/',
    fixture: 'resets',
    ready: '[aria-labelledby=resets-heading] [data-use-reset]',
    steps: [{ click: '[aria-label="Use Full reset on Main"] >> nth=0' }, { waitFor: "text=This can't be undone." }],
    viewport: { width: 1600, height: 900 },
  },
  {
    // Used: the row turns to dust and the list closes the gap, under the page's policy (the fragments' styles are
    // set from script); then the list without it, and focus on the next row's Use.
    name: 'overview-reset-used',
    path: '/',
    fixture: 'resetsUsed',
    ready: '[aria-labelledby=resets-heading] [data-use-reset]',
    steps: [
      { click: '[aria-label="Use 5-hour reset on Main"]' },
      { click: '[role=dialog] button[type=submit]' },
      { waitFor: '[role=status] >> text=5-hour reset used on Main.' },
      // The row has turned to dust and left the list: the screenshot shows the list without it.
      { waitGone: '[aria-labelledby=resets-heading] [data-gone]' },
    ],
    viewport: { width: 1600, height: 900 },
  },
  {
    // Mid-way: the used row half turned to dust, the dust bunnies uncovered where it was (a still, every animation
    // paused 1.4 s in), under the page's policy; and the same in the desktop window, under the window's policy.
    name: 'overview-reset-dust',
    path: '/',
    fixture: 'resetsUsed',
    ready: '[aria-labelledby=resets-heading] [data-use-reset]',
    steps: [
      { click: '[aria-label="Use 5-hour reset on Main"]' },
      // The dust bunnies, the first of the characters.
      { random: 0 },
      { click: '[role=dialog] button[type=submit]' },
      { waitFor: '[aria-labelledby=resets-heading] .dust-hole.dust-art-bunnies' },
      { masked: '[aria-labelledby=resets-heading] .dust-hole.dust-art-bunnies', drawing: 'dust-bunnies' },
      { freeze: 1400 },
    ],
    viewport: { width: 1600, height: 900 },
  },
  {
    name: 'desktop-reset-dust',
    path: '/?desktop',
    fixture: 'resetsUsed',
    ready: '[aria-labelledby=resets-heading] [data-use-reset]',
    steps: [
      { click: '[aria-label="Use 5-hour reset on Main"]' },
      // Cthulhu, the second (its drawing under the desktop window's policy too).
      { random: 0.5 },
      { click: '[role=dialog] button[type=submit]' },
      { waitFor: '[aria-labelledby=resets-heading] .dust-hole.dust-art-cthulhu' },
      { masked: '[aria-labelledby=resets-heading] .dust-hole.dust-art-cthulhu', drawing: 'cthulhu' },
      { freeze: 1400 },
    ],
    viewport: { width: 1600, height: 900 },
  },
  {
    // The provider said no: its answer in the modal, the reset still listed.
    name: 'overview-use-reset-refused',
    path: '/',
    fixture: 'resetsRefused',
    ready: '[aria-labelledby=resets-heading] [data-use-reset]',
    steps: [
      { click: '[aria-label="Use Full reset on Main"] >> nth=0' },
      { click: '[role=dialog] button[type=submit]' },
      { waitFor: '[data-use-reset-error]' },
    ],
    viewport: { width: 1600, height: 900 },
  },
  {
    name: 'overview-hooks-off',
    path: '/',
    fixture: 'hooksOff',
    ready: 'text=Hooks are off for this run',
  },
  {
    name: 'overview-filtered',
    path: '/',
    fixture: 'normal',
    ready: 'main [role=table]',
    steps: [
      {
        click: 'button[role=checkbox][aria-label$="of Second"]',
      },
      {
        waitFor: 'text=Show all',
      },
    ],
  },
  {
    name: 'overview-core-not-running',
    path: '/',
    fixture: 'offline',
    ready: 'main p',
  },
  {
    name: 'overview-signed-in-as-expected',
    path: '/',
    fixture: 'normal',
    ready: 'main [role=table]',
    steps: [
      {
        focus: 'button:has-text("Signed in as")',
      },
      {
        waitFor: 'text=Expected sam@example.com',
      },
    ],
  },
  {
    name: 'settings-general',
    path: '/#/settings/general',
    fixture: 'normal',
    ready: 'text=Warn before a reset expires',
  },
  {
    name: 'settings-accounts-signed-in-as-expected',
    path: '/#/settings/accounts',
    fixture: 'normal',
    ready: '[aria-label=Accounts] [role=row]',
    steps: [
      {
        focus: '[aria-label=Accounts] button:has-text("Signed in as")',
      },
      {
        waitFor: 'text=Expected sam@example.com',
      },
    ],
  },
  {
    name: 'settings-accounts',
    path: '/#/settings/accounts',
    fixture: 'healthy',
    ready: '[aria-label=Accounts] [role=row]',
  },
  {
    name: 'settings-accounts-hooks',
    path: '/#/settings/accounts',
    fixture: 'hooks',
    ready: '[data-hooks] >> text=Still installed',
  },
  {
    name: 'settings-accounts-work-hours',
    path: '/#/settings/accounts',
    fixture: 'workHours',
    ready: '[data-work-hours]',
  },
  {
    name: 'settings-accounts-work-hours-pressed',
    path: '/#/settings/accounts',
    fixture: 'workHours',
    ready: '[data-work-hours]',
    steps: [{ press: '[role=slider][aria-label="Work hours end"]' }],
  },
  {
    name: 'settings-accounts-reserve-pressed',
    path: '/#/settings/accounts',
    fixture: 'normal',
    ready: '#reserve-label',
    steps: [{ press: '[role=slider][aria-labelledby=reserve-label]' }],
  },
  {
    name: 'settings-accounts-switch-off',
    path: '/#/settings/accounts',
    fixture: 'normal',
    ready: '[aria-label=Accounts] [role=switch]',
    steps: [{ hover: '[role=switch][aria-label="Use Codex personal"]' }],
  },
  {
    name: 'settings-add-account',
    path: '/#/settings/accounts/add',
    fixture: 'normal',
    ready: '#add-name',
  },
  {
    name: 'add-account-alone',
    path: '/',
    fixture: 'normal',
    ready: 'main [role=table]',
    steps: [{ click: 'aside button[aria-label="Add account"]' }, { waitFor: '#add-name' }],
  },
  {
    name: 'sidebar-add-account-hover',
    path: '/',
    fixture: 'normal',
    ready: 'main [role=table]',
    steps: [{ hover: 'aside button[aria-label="Add account"]' }],
  },
  {
    name: 'sidebar-add-account-press',
    path: '/',
    fixture: 'normal',
    ready: 'main [role=table]',
    steps: [{ press: 'aside button[aria-label="Add account"]' }],
  },
  {
    name: 'settings-add-existing-folder',
    path: '/#/settings/accounts/add',
    fixture: 'normal',
    ready: '#add-name',
    steps: [
      {
        click: '[role=radio]:has-text("Existing folder")',
      },
      {
        waitFor: '#add-folder',
      },
    ],
  },
  {
    name: 'settings-rename-account',
    path: '/#/settings/accounts',
    fixture: 'normal',
    ready: '[aria-label=Accounts] [role=row]',
    steps: [
      {
        click: 'button[aria-label="More options for Second"]',
      },
      {
        click: '[role=menuitem]:has-text("Rename")',
      },
      {
        waitFor: '#rename-name',
      },
    ],
  },
  {
    name: 'settings-remove-account',
    path: '/#/settings/accounts',
    fixture: 'normal',
    ready: '[aria-label=Accounts] [role=row]',
    steps: [
      {
        click: 'button[aria-label="More options for Second"]',
      },
      {
        click: '[role=menuitem]:has-text("Remove")',
      },
      {
        waitFor: 'text=Remove Second?',
      },
    ],
  },
  {
    name: 'settings-accounts-menu',
    path: '/#/settings/accounts',
    fixture: 'normal',
    ready: '[aria-label=Accounts] [role=row]',
    steps: [
      {
        click: 'button[aria-label="More options for Second"]',
      },
      {
        waitFor: '[role=menuitem]',
      },
    ],
  },
  {
    name: 'settings-search',
    path: '/#/settings/general',
    fixture: 'normal',
    ready: 'text=Read usage every',
    steps: [
      {
        fill: 'input[type=search]',
        text: 'account',
      },
    ],
  },
  {
    name: 'first-run-setup',
    path: '/',
    fixture: 'setup',
    ready: 'section[aria-labelledby=setup-others] [role=table]',
  },
  {
    name: 'first-run-setup-small',
    path: '/',
    fixture: 'setupFive',
    ready: 'section[aria-labelledby=setup-others] [role=table]',
    viewport: { width: 800, height: 560 },
    // The first candidate shows without scrolling.
    steps: [{ inView: '#setup-use-0', within: '[data-setup-body]' }],
  },
  {
    name: 'desktop-overview',
    path: '/?desktop',
    fixture: 'desktop',
    ready: 'main [role=table]',
  },
  {
    name: 'desktop-app-menu',
    path: '/?desktop',
    fixture: 'desktop',
    ready: 'main [role=table]',
    steps: [
      {
        click: '[aria-label="Warden menu"]',
      },
      {
        waitFor: '[role=menuitem]',
      },
    ],
  },
  {
    // The same menu opened from the keyboard: its first item, Settings, has focus and shows the focus outline.
    name: 'desktop-app-menu-keyboard',
    path: '/?desktop',
    fixture: 'desktop',
    ready: 'main [role=table]',
    steps: [{ focus: '[aria-label="Warden menu"]' }, { key: 'Enter' }, { waitFor: '[role=menuitem]:focus-visible' }],
  },
  {
    // Settings chosen from the menu with the keyboard: it opens in General once the menu has closed, focus inside it
    // (with the menu's closing animation, which the unit tests do not have).
    name: 'desktop-app-menu-settings',
    path: '/?desktop',
    fixture: 'desktop',
    ready: 'main [role=table]',
    steps: [
      { focus: '[aria-label="Warden menu"]' },
      { key: 'Enter' },
      { waitFor: '[role=menuitem]:focus-visible' },
      { key: 'Enter' },
      { waitFor: '[role=dialog] h2 >> text=General' },
      { focused: ':is([role=dialog], [role=dialog] *)' },
      // Still there once the menu has gone: nothing took focus back from Settings.
      { waitGone: '[role=menu]' },
      { focused: ':is([role=dialog], [role=dialog] *)' },
    ],
  },
  {
    // Accounts chosen the same way, then Escape: focus is back on the menu's button (the item it came from is gone).
    name: 'desktop-app-menu-accounts-closed',
    path: '/?desktop',
    fixture: 'desktop',
    ready: 'main [role=table]',
    steps: [
      { focus: '[aria-label="Warden menu"]' },
      { key: 'Enter' },
      { waitFor: '[role=menuitem]:focus-visible' },
      { key: 'ArrowDown' },
      { key: 'Enter' },
      { waitFor: '[role=dialog] h2 >> text=Accounts' },
      { focused: ':is([role=dialog], [role=dialog] *)' },
      { waitGone: '[role=menu]' },
      { focused: ':is([role=dialog], [role=dialog] *)' },
      { key: 'Escape' },
      { waitGone: '[role=dialog]' },
      { focused: '[aria-label="Warden menu"]' },
    ],
  },
  {
    name: 'desktop-close-hovered',
    path: '/?desktop',
    fixture: 'desktop',
    ready: 'main [role=table]',
    steps: [
      {
        hover: 'button[aria-label=Close]',
      },
    ],
  },
  {
    name: 'desktop-close-pressed',
    path: '/?desktop',
    fixture: 'desktop',
    ready: 'main [role=table]',
    steps: [
      {
        press: 'button[aria-label=Close]',
      },
    ],
  },
  {
    name: 'desktop-settings-general',
    path: '/?desktop#/settings/general',
    fixture: 'desktop',
    ready: 'text=Warn before a reset expires',
  },
  {
    name: 'desktop-update-downloading',
    path: '/?desktop',
    fixture: 'desktopDownloading',
    ready: '[role=progressbar]',
  },
  {
    name: 'desktop-update-ready',
    path: '/?desktop',
    fixture: 'desktopUpdate',
    ready: 'button[aria-label="Install and restart, Warden 0.2.0"]',
  },
  {
    name: 'desktop-update-ready-pressed',
    path: '/?desktop',
    fixture: 'desktopUpdate',
    ready: 'button[aria-label="Install and restart, Warden 0.2.0"]',
    steps: [
      {
        press: 'button[aria-label="Install and restart, Warden 0.2.0"]',
      },
    ],
  },
  {
    name: 'desktop-update-confirm',
    path: '/?desktop',
    fixture: 'desktopUpdate',
    ready: 'button[aria-label="Install and restart, Warden 0.2.0"]',
    steps: [
      {
        click: 'button[aria-label="Install and restart, Warden 0.2.0"]',
      },
      {
        waitFor: 'text=Warden closes, installs the update and starts again.',
      },
    ],
  },
  {
    name: 'desktop-update-installing',
    path: '/?desktop',
    fixture: 'desktopUpdate',
    ready: 'button[aria-label="Install and restart, Warden 0.2.0"]',
    steps: [
      {
        click: 'button[aria-label="Install and restart, Warden 0.2.0"]',
      },
      {
        click: 'button[type=submit]:has-text("Install and restart")',
      },
      {
        waitFor: '[role=status]:has-text("Installing Warden 0.2.0")',
      },
    ],
  },
  {
    name: 'desktop-add-existing-folder',
    path: '/?desktop#/settings/accounts/add',
    fixture: 'desktop',
    ready: '#add-name',
    steps: [
      {
        click: '[role=radio]:has-text("Existing folder")',
      },
      {
        waitFor: '#add-folder',
      },
    ],
  },
];

/**
 * Stands in for the desktop app's Tauri bridge on `?desktop` pages: the window is neither maximized nor minimized,
 * and every other call answers nothing.
 */
function fakeTauri(): void {
  if (!new URLSearchParams(location.search).has('desktop')) return;
  let next = 1;
  Object.assign(globalThis, {
    isTauri: true,
    __TAURI_INTERNALS__: {
      metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
      transformCallback: () => next++,
      unregisterCallback: () => undefined,
      invoke: (cmd: string) =>
        Promise.resolve(cmd.includes('is_maximized') || cmd.includes('is_minimized') ? false : null),
    },
  });
}

const THEMES = ['light', 'dark'] as const;
const VIEWPORT = { width: 1280, height: 800 };
const AXE_TAGS = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa', 'best-practice'];

interface AxeResult {
  violations: { id: string; impact: string | null; help: string; nodes: { target: string[] }[] }[];
}

/** This run's made-up token and its cookie, as the demo server would have them on the preview's port. */
const TOKEN = 'a11ya11ya11y';
const COOKIE = 'warden-token-a11y';
/** Where the desktop window's page lives (ui/src/api.ts, `DESKTOP_ORIGIN`). */
const DESKTOP_ORIGIN = 'http://warden.localhost';

/**
 * As the demo server: `?token=` trades the token for a cookie and leaves the address; anything without the
 * cookie is refused. Also serves axe from the page's own origin, since the policy refuses inline scripts.
 */
function devServerToken(axeSource: string): Plugin {
  return {
    name: 'warden-fake-token',
    configurePreviewServer(server) {
      server.middlewares.use((req, res, next) => {
        const url = new URL(req.url ?? '/', 'http://127.0.0.1');
        if (url.searchParams.get('token') === TOKEN) {
          url.searchParams.delete('token');
          res.statusCode = 303;
          res.setHeader('Location', url.pathname + url.search);
          res.setHeader('Set-Cookie', `${COOKIE}=${TOKEN}; Path=/; HttpOnly; SameSite=Strict`);
          res.end();
          return;
        }
        if (!(req.headers.cookie ?? '').split(';').some((c) => c.trim() === `${COOKIE}=${TOKEN}`)) {
          res.statusCode = 403;
          res.end();
          return;
        }
        if (url.pathname === '/__axe.js') {
          res.setHeader('Content-Type', 'text/javascript');
          res.end(readFileSync(axeSource));
          return;
        }
        next();
      });
    },
  };
}

/** Answers /api/ from the fixture the current page uses. The long poll is left open; leaving the page drops it. */
function fakeApi(current: () => Fixture): Plugin {
  const send = (res: ServerResponse, body: unknown) => {
    res.setHeader('Content-Type', 'application/json');
    res.end(JSON.stringify(body));
  };
  return {
    name: 'warden-fake-api',
    configurePreviewServer(server) {
      server.middlewares.use('/api', (req, res) => {
        const fixture = current();
        if (fixture === 'offline') {
          res.statusCode = 502;
          res.end();
          return;
        }
        const now = Date.now();
        const state =
          fixture === 'setup' ||
          fixture === 'setupSignedOut' ||
          fixture === 'setupOtherAccount' ||
          fixture === 'setupFive'
            ? setupState(now)
            : fixture === 'desktop' || fixture === 'desktopDownloading' || fixture === 'desktopUpdate'
              ? desktopState(now)
              : fixture === 'tree'
                ? treeState(now)
                : fixture === 'quietHidden'
                  ? quietHiddenState(now)
                  : fixture === 'healthy'
                    ? healthyState(now)
                    : fixture === 'resets' || fixture === 'resetsRefused' || fixture === 'resetsUsed'
                      ? resetsState(now)
                      : normalState(now);
        if (fixture === 'hooksOff') state.hooksOff = true;
        if (fixture === 'desktopDownloading') state.update = { version: '0.2.0', downloaded: 45, ready: false };
        if (fixture === 'desktopUpdate') state.update = { version: '0.2.0', downloaded: 100, ready: true };
        if (fixture === 'setupSignedOut' && state.main) {
          state.main.identity = null;
          state.problems = [{ kind: 'claudeAppSignedOut' }];
          state.main.signIn = { state: 'signedOut', email: null };
        }
        if (fixture === 'setupOtherAccount' && state.main)
          state.main.signIn = { state: 'otherAccount', email: 'sam@home.example' };
        if (fixture === 'mainSignedOut' && state.main) {
          state.main.signIn = { state: 'signedOut', email: null };
          state.main.usage = null;
          state.main.usageError = { kind: 'signedOut' };
        }
        if (fixture === 'mainExpired' && state.main) {
          state.main.signIn = { state: 'expired', email: state.main.identity!.email };
          state.main.usageError = { kind: 'loginExpired' };
        }
        if (fixture === 'mainOtherAccount' && state.main) {
          state.main.signIn = { state: 'otherAccount', email: 'other@example.com' };
          state.main.usage = null;
          state.main.usageError = { kind: 'identityChanged' };
        }
        if (fixture === 'mainHooksMissing' && state.main) state.main.hooks = 'notInstalled';
        if (fixture === 'accountHooksMissing') state.accounts[0]!.hooks = 'notInstalled';
        if (fixture === 'problems') problemsState(state);
        if (fixture === 'hooks') hooksState(state);
        if (fixture === 'workHours') state.settings.workHours = { days: [1, 2, 3, 4, 5], from: 540, to: 1080 };
        if (fixture === 'delegated')
          state.delegation = {
            ...state.delegation,
            mainWorksItself: false,
            mainReason: 'Main is near its reserve (86% of its week; it keeps 14% until Sun 14:15).',
          };
        if (fixture === 'noTarget')
          state.delegation = {
            mainWorksItself: true,
            delegated: null,
            mainReason: 'No account can take delegated work, so main keeps it.',
            delegatedReason: 'No account can take delegated work, so main keeps it.',
            handOffAt: 84,
          };
        if (fixture === 'mainUnknown' && state.main) {
          state.main.signIn = { state: 'signedOut', email: null };
          state.main.usage = null;
          state.main.usageError = { kind: 'signedOut' };
          state.delegation = {
            ...state.delegation,
            mainWorksItself: false,
            mainReason:
              "Main's usage is unknown, so work goes to the other accounts until Warden is signed in to main.",
          };
        }
        const url = req.url ?? '';
        if (url.startsWith('/state/wait')) return;
        // Using a reset: refused as the provider would say it (nothing real is sent; the other fixtures answer the state).
        if (fixture === 'resetsRefused' && req.method === 'POST' && /\/resets\/use$/.test(url)) {
          res.statusCode = 409;
          send(res, { error: 'This reset was already used.' });
          return;
        }
        if (fixture === 'resetsUsed' && req.method === 'POST' && /\/resets\/use$/.test(url) && state.main) {
          state.main.resets = state.main.resets.filter((r) => r.id !== 'grant-session');
        }
        if (url.startsWith('/setup/candidates')) {
          const known = new Set(state.accounts.map((a) => a.folder));
          const found = fixture === 'setupFive' ? fiveCandidates : candidates;
          send(
            res,
            found.map((c) => ({ ...c, known: known.has(c.folder) })),
          );
          return;
        }
        send(res, state);
      });
    },
  };
}

async function main(): Promise<void> {
  const baseIndex = process.argv.indexOf('--base');
  const pages =
    baseIndex < 0 ? PAGES : affectedPages(PAGES, changedFiles(process.argv[baseIndex + 1] ?? 'master', true).files);
  console.log(
    `a11y: selected ${pages.length}/${PAGES.length} states: ${pages.map((p) => p.name).join(', ') || '(none)'}`,
  );
  if (process.argv.includes('--dry-run') || !pages.length) return;
  const uiRoot = join(ROOT, 'ui');
  const shots = join(ROOT, 'artifacts', 'screenshots');
  mkdirSync(shots, { recursive: true });

  await build({ root: uiRoot, logLevel: 'warn' });
  const axeSource = createRequire(import.meta.url).resolve('axe-core/axe.min.js');
  let fixture: Fixture = 'normal';
  const server = await preview({
    root: uiRoot,
    logLevel: 'warn',
    preview: { host: '127.0.0.1', port: 0, headers: { 'Content-Security-Policy': browserCsp() } },
    plugins: [devServerToken(axeSource), fakeApi(() => fixture)],
  });
  const base = server.resolvedUrls?.local[0];
  if (!base) throw new Error('vite preview did not report its address');
  const windowCsp = desktopCsp();

  const browser = await chromium.launch({ channel: 'msedge' });
  let failures = 0;
  try {
    for (const theme of THEMES) {
      const context = await browser.newContext({ viewport: VIEWPORT, colorScheme: theme });
      // The desktop window's page and API, passed on to the preview server with the window's own policy.
      await context.route(`${DESKTOP_ORIGIN}/**`, async (route) => {
        const url = new URL(route.request().url());
        try {
          const response = await route.fetch({
            url: new URL(url.pathname + url.search, base).href,
            headers: { ...route.request().headers(), cookie: `${COOKIE}=${TOKEN}` },
          });
          await route.fulfill({
            response,
            headers: { ...response.headers(), 'content-security-policy': windowCsp },
          });
        } catch {
          // The long poll, dropped when the page is left.
          await route.abort().catch(() => undefined);
        }
      });
      const page = await context.newPage();
      await page.addInitScript(fakeTauri);
      let blocked: string[] = [];
      page.on('console', (m) => {
        if (/Content Security Policy/i.test(m.text())) blocked.push(m.text());
      });
      // The printed address: the token becomes a cookie and leaves the address bar.
      await page.goto(new URL(`/?token=${TOKEN}`, base).href, { waitUntil: 'load' });
      if (new URL(page.url()).search !== '') throw new Error(`the token stayed in the address: ${page.url()}`);
      for (const p of pages) {
        fixture = p.fixture;
        blocked = [];
        await page.goto('about:blank');
        await page.setViewportSize(p.viewport ?? VIEWPORT);
        const origin = p.path.includes('?desktop') ? DESKTOP_ORIGIN : base;
        await page.goto(new URL(p.path, origin).href, { waitUntil: 'load' });
        await page.locator(p.ready).first().waitFor({ timeout: 10_000 });
        for (const step of p.steps ?? []) {
          if ('click' in step) await page.locator(step.click).click({ timeout: 10_000 });
          else if ('focus' in step) {
            // Scrolled into view first, and the scroll let settle: a tooltip closes when the page scrolls under it.
            const target = page.locator(step.focus);
            await target.scrollIntoViewIfNeeded({ timeout: 10_000 });
            await page.waitForTimeout(500);
            await target.focus({ timeout: 10_000 });
          } else if ('hover' in step) await page.locator(step.hover).hover({ timeout: 10_000 });
          else if ('press' in step) {
            await page.locator(step.press).hover({ timeout: 10_000 });
            await page.mouse.down();
          } else if ('key' in step) await page.keyboard.press(step.key);
          else if ('focused' in step) {
            const ok = await page
              .waitForFunction((sel) => document.activeElement?.matches(sel) ?? false, step.focused, {
                timeout: 10_000,
              })
              .then(
                () => true,
                () => false,
              );
            if (!ok) {
              const has = await page.evaluate(() => document.activeElement?.outerHTML.slice(0, 120) ?? 'nothing');
              throw new Error(`${p.name}: focus is not on ${step.focused} (it is on ${has})`);
            }
          } else if ('fill' in step) await page.locator(step.fill).fill(step.text, { timeout: 10_000 });
          else if ('inView' in step) {
            const shown = await page.evaluate(
              ([el, area]) => {
                const e = document.querySelector(el!)?.closest('[role=row]')?.getBoundingClientRect();
                const a = document.querySelector(area!)?.getBoundingClientRect();
                return !!e && !!a && e.top >= a.top && e.bottom <= a.bottom;
              },
              [step.inView, step.within],
            );
            if (!shown) throw new Error(`${p.name}: ${step.inView} is not wholly visible in ${step.within}`);
          } else if ('masked' in step) {
            // The hole's ::before has the expected drawing as its mask (the build names it <drawing>-<hash>.svg), and
            // that drawing loads and decodes under the page's policy.
            const { mask, url, decoded } = await page.evaluate(async (sel) => {
              const el = document.querySelector(sel);
              const mask = el ? getComputedStyle(el, '::before').maskImage : 'missing';
              const url = /^url\("?(.*?)"?\)$/.exec(mask)?.[1];
              if (!url) return { mask, url: '', decoded: false };
              const img = new Image();
              img.src = url;
              return {
                mask,
                url,
                decoded: await img.decode().then(
                  () => true,
                  () => false,
                ),
              };
            }, step.masked);
            const file = url ? new URL(url, page.url()).pathname.split('/').pop()! : '';
            if (!new RegExp(`^${step.drawing}-[\\w-]+\\.svg$`).test(file))
              throw new Error(`${step.masked}::before is not masked with ${step.drawing}.svg (${mask})`);
            if (!decoded) throw new Error(`${step.masked}::before: ${file} does not load or decode`);
          } else if ('random' in step) {
            await page.evaluate((value) => {
              const real = Math.random;
              Math.random = () => {
                Math.random = real;
                return value;
              };
            }, step.random);
          } else if ('freeze' in step) {
            await page.evaluate((at) => {
              // Remembered, so the settle wait below skips these and no others.
              const frozen = new Set<Animation>();
              for (const a of document.getAnimations()) {
                a.pause();
                a.currentTime = at;
                frozen.add(a);
              }
              (window as unknown as { checkFrozen?: Set<Animation> }).checkFrozen = frozen;
            }, step.freeze);
          } else if ('waitGone' in step) {
            await page.locator(step.waitGone).first().waitFor({ state: 'detached', timeout: 10_000 });
          } else await page.locator(step.waitFor).first().waitFor({ timeout: 10_000 });
        }
        await page.evaluate(() => document.fonts.ready);
        await page.waitForTimeout(300);
        // Modals and menus animate open; check them once they have settled (spinners turn for ever, so not those, nor an
        // animation a `freeze` step paused for a still). One that does not settle fails the state rather than hanging.
        // Any other paused animation never finishes, so it fails the state.
        const settled = await page.evaluate(() => {
          const frozen = (window as unknown as { checkFrozen?: Set<Animation> }).checkFrozen;
          return Promise.race([
            Promise.all(
              document
                .getAnimations()
                .filter((a) => !frozen?.has(a) && a.effect?.getComputedTiming().iterations !== Infinity)
                .map((a) => a.finished.catch(() => undefined)),
            ).then(() => true),
            new Promise<boolean>((r) => setTimeout(() => r(false), 15_000)),
          ]);
        });
        if (!settled) {
          failures++;
          console.error(`FAIL ${p.name} (${theme}): its animations did not settle within 15 s`);
        }
        // Only the app's own areas scroll (the content panel, a modal's body); the page itself never does.
        const overflow = await page.evaluate(() => {
          const d = document.scrollingElement ?? document.documentElement;
          return { sh: d.scrollHeight, h: window.innerHeight, sw: d.scrollWidth, w: window.innerWidth };
        });
        if (overflow.sh > overflow.h || overflow.sw > overflow.w) {
          failures++;
          console.error(
            `FAIL ${p.name} (${theme}): the page itself can scroll (document ${overflow.sw} × ${overflow.sh}, window ${overflow.w} × ${overflow.h})`,
          );
        }
        await page.screenshot({ path: join(shots, `${p.name}-${theme}.png`), fullPage: true });
        await page.addScriptTag({ url: '/__axe.js' });
        const result = (await page.evaluate(
          (tags) =>
            (globalThis as unknown as { axe: { run: (c: unknown, o: unknown) => Promise<unknown> } }).axe.run(
              document,
              { runOnly: tags },
            ),
          AXE_TAGS,
        )) as AxeResult;
        for (const v of result.violations) {
          failures++;
          console.error(`FAIL ${p.name} (${theme}): ${v.id} [${v.impact}] ${v.help}`);
          for (const node of v.nodes.slice(0, 5)) console.error(`     at ${node.target.join(' ')}`);
        }
        for (const text of blocked) {
          failures++;
          console.error(`FAIL ${p.name} (${theme}): the Content-Security-Policy blocked something: ${text}`);
        }
        // Let go and move the pointer off to the window's corner, so no hover or press carries to the next page.
        await page.mouse.up();
        await page.mouse.move(0, 0);
      }
      await context.close();
    }
  } finally {
    await browser.close();
    const http = server.httpServer;
    if ('closeAllConnections' in http) http.closeAllConnections();
    await new Promise<void>((resolve) => server.httpServer.close(() => resolve()));
  }
  console.log(
    `a11y: ${pages.length} page(s) x ${THEMES.length} themes, ${failures} violation(s); screenshots in artifacts/screenshots/`,
  );
  if (failures) process.exitCode = 1;
}

if (import.meta.main) await main();
