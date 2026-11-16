import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { store } from '../store';
import type { Reset, Session } from '../api';
import { fakeServer } from '../test/api';
import { NOW, account, state } from '../test/state';
import OverviewPage from './OverviewPage.vue';

const HOUR = 3_600_000;

afterEach(() => {
  store.state = null;
  store.connected = true;
  vi.unstubAllGlobals();
});

describe('OverviewPage', () => {
  it('says when the core is not running', () => {
    store.connected = false;
    const page = mount(OverviewPage);
    expect(page.get('h1').text()).toBe('Overview');
    expect(page.text()).toContain("Warden's core is not running");
  });

  it('lists the accounts in use with their limits, and leaves switched-off accounts out', () => {
    store.now = NOW;
    store.state = state([
      account({
        id: 'main',
        label: 'Main',
        usage: {
          readAt: NOW - 2 * 60_000,
          fiveHour: { usedPercent: 61, resetsAt: NOW + 2 * HOUR },
          weekly: { usedPercent: 85, resetsAt: NOW + 30 * HOUR },
          perModelWeekly: [],
          extra: null,
        },
      }),
      account({ id: 'c', label: 'Codex', provider: 'codex', usageError: { kind: 'noLogs' } }),
      account({ id: 'p', label: 'Personal', enabled: false }),
    ]);
    const page = mount(OverviewPage);
    const rows = page.findAll('[aria-labelledby=accounts-heading] [role=row]');
    expect(rows).toHaveLength(3);
    expect(rows[1]!.text()).toContain('61% used');
    expect(rows[1]!.text()).toContain('85% used');
    // A fresh reading needs no note; the Reading column is gone.
    expect(rows[1]!.text()).not.toContain('Read 2 min ago');
    expect(page.findAll('[role=columnheader]').map((h) => h.text())).not.toContain('Reading');
    expect(rows[2]!.text()).toContain('No Codex sessions yet');
    expect(page.text()).not.toContain('Personal');
  });

  it('notes an old reading in grey, and shows an idle Warden login as grey bars with its age and no amber', () => {
    store.now = NOW;
    const usage = (readAt: number) => ({
      readAt,
      fiveHour: { usedPercent: 10, resetsAt: NOW + HOUR },
      weekly: { usedPercent: 20, resetsAt: NOW + 30 * HOUR },
      perModelWeekly: [],
      extra: null,
    });
    store.state = state([
      account({ id: 'main', label: 'Main', usage: usage(NOW - 3 * HOUR) }),
      account({
        id: 's',
        label: 'Second',
        usage: usage(NOW - 60_000),
        usageError: { kind: 'loginExpired' },
      }),
    ]);
    const rows = mount(OverviewPage).findAll('[aria-labelledby=accounts-heading] [role=row]');
    expect(rows[1]!.text()).toContain('Read 3 h ago');
    expect(rows[1]!.find('.text-attn').exists()).toBe(false);
    // Second's login is idle: no amber, its bars grey, its age with the tooltip (and for screen readers).
    expect(rows[2]!.find('.text-attn').exists()).toBe(false);
    expect(rows[2]!.findAll('.bg-bar-idle')).toHaveLength(2);
    expect(rows[2]!.text()).toContain('Read 1 min ago');
    // The age takes focus in the 5-hour cell, and its tooltip says why the bars are grey.
    const age = rows[2]!.findAll('button').find((b) => b.text() === 'Read 1 min ago')!;
    expect(age.attributes('tabindex')).not.toBe('-1');
    expect(age.element.closest('[aria-hidden=true]')).toBeNull();
    expect(age.element.closest('[role=cell]')).not.toBeNull();
    const tips = mount(OverviewPage)
      .findAllComponents({ name: 'WTooltip' })
      .map((w) => w.props('text'));
    expect(tips).toContain(
      'Its login is idle. The last reading still counts, and Claude renews the login the next time this account works.',
    );
    // Main's own expired sign-in stays amber.
    store.state.main!.usageError = { kind: 'loginExpired' };
    const again = mount(OverviewPage).findAll('[aria-labelledby=accounts-heading] [role=row]');
    expect(again[1]!.get('.text-attn').text()).toBe("Sign in again for main's usage");
    expect(again[1]!.findAll('.bg-bar-idle')).toHaveLength(0);
  });

  it("marks main's hand-off point on its weekly bar, and says it for screen readers", () => {
    store.now = NOW;
    const usage = {
      readAt: NOW,
      fiveHour: { usedPercent: 10, resetsAt: NOW + HOUR },
      weekly: { usedPercent: 40, resetsAt: NOW + 30 * HOUR },
      perModelWeekly: [],
      extra: null,
    };
    store.state = state([account({ id: 'main', label: 'Main', usage }), account({ id: 's', label: 'Second', usage })]);
    store.state.delegation.handOffAt = 98;
    const rows = mount(OverviewPage).findAll('[aria-labelledby=accounts-heading] [role=row]');
    const marks = rows[1]!.findAll('[data-mark]');
    expect(marks).toHaveLength(1);
    expect(marks[0]!.attributes('style')).toContain('left: 98%');
    expect(rows[1]!.text()).toContain('Hands off at 98%.');
    expect(rows[2]!.findAll('[data-mark]')).toHaveLength(0);
    // At 100% there is nothing to mark.
    store.state.delegation.handOffAt = 100;
    expect(mount(OverviewPage).findAll('[data-mark]')).toHaveLength(0);
  });

  it('has no "Delegated work" line under the accounts, and no "Main" under a renamed main (its crown says it)', () => {
    store.state = state([account({ id: 'main', label: 'Home' }), account({ id: 's', label: 'Second' })]);
    const page = mount(OverviewPage);
    const rows = page.findAll('[aria-labelledby=accounts-heading] [role=row]');
    expect(rows[1]!.findAll('[role=cell]')[1]!.text()).not.toContain('Main');
    expect(rows[1]!.find('[aria-label="Main account"]').exists()).toBe(true);
    expect(rows[2]!.text()).not.toContain('Delegated work');
    // No hint under the Accounts heading, nor "All accounts" beside Sessions while nothing is checked.
    expect(page.text()).not.toContain('Check accounts to show only their sessions.');
    expect(page.text()).not.toContain('All accounts');
  });

  it('shows a changed sign-in in amber words', () => {
    store.state = state([
      account({
        id: 'main',
        label: 'Main',
        identityChange: { now: { email: 'me@home.example', organization: null }, knownAccountId: null },
      }),
    ]);
    const page = mount(OverviewPage);
    expect(page.find('.text-attn').text()).toBe('Signed in as me@home.example');
  });

  it('shows each account’s email under its name, not only on hover', () => {
    store.state = state([
      account({ id: 'main', label: 'Main', identity: { email: 'ann@example.com', organization: null } }),
    ]);
    const row = mount(OverviewPage).findAll('[aria-labelledby=accounts-heading] [role=row]')[1]!;
    expect(row.text()).toContain('Main');
    expect(row.text()).toContain('ann@example.com');
  });

  it('shows Extra usage only for spend above zero, and drops the column while no account has any', () => {
    const extra = (usedCredits: number) => ({
      readAt: NOW,
      fiveHour: null,
      weekly: null,
      perModelWeekly: [],
      extra: { enabled: true, usedCredits, monthlyLimit: 5000, usedPercent: usedCredits / 50 },
    });
    store.state = state([
      account({ id: 'main', label: 'Main', usage: extra(0) }),
      account({ id: 's', label: 'Second', usage: extra(0) }),
    ]);
    expect(mount(OverviewPage).text()).not.toContain('Extra usage');

    store.state = state([
      account({ id: 'main', label: 'Main', usage: extra(1240) }),
      account({ id: 's', label: 'Second', usage: extra(0) }),
    ]);
    const page = mount(OverviewPage);
    expect(page.text()).toContain('Extra usage');
    const rows = page.findAll('[aria-labelledby=accounts-heading] [role=row]');
    expect(rows[1]!.text()).toContain('$12.40 of $50');
    expect(rows[2]!.text()).not.toContain('$');
  });

  function session(s: Partial<Session> & Pick<Session, 'id' | 'accountId' | 'state'>): Session {
    return {
      parentId: null,
      provider: 'claude',
      doing: 'Editing files',
      project: 'shop',
      name: `Session ${s.id}`,
      startedAt: NOW - 30 * 60_000,
      lastEventAt: NOW,
      endedAt: null,
      subagents: [],
      signedInAs: null,
      source: 'hook',
      ...s,
    };
  }

  function withSessions() {
    store.now = NOW;
    const st = state([
      account({ id: 'main', label: 'Main' }),
      account({ id: 'x', label: 'Second' }),
      account({ id: 'p', label: 'Personal', enabled: false }),
    ]);
    st.sessions = [
      session({ id: 'w1', accountId: 'main', state: 'working', subagents: [{ type: 'Explore' }, { type: 'Plan' }] }),
      session({ id: 'i1', accountId: 'main', state: 'idle', doing: 'Waiting for your next message' }),
      session({
        id: 'a1',
        accountId: 'x',
        state: 'waiting',
        doing: 'Asks to run a command',
        name: '#55402: Fix login',
      }),
      session({ id: 'off', accountId: 'p', state: 'working' }),
      session({ id: 'old', accountId: 'x', state: 'done', doing: 'Ended', endedAt: NOW - 20 * 60_000 }),
    ];
    st.activity = [
      { accountId: 'x', bins: [{ at: NOW - (NOW % 300_000) - 600_000, working: 2, last: 1, waited: true }] },
    ];
    store.state = st;
    return mount(OverviewPage);
  }

  const sessionRows = (page: ReturnType<typeof mount>) =>
    page.findAll('[aria-labelledby=sessions-heading] [role=row]').slice(1);

  it('nests each delegated session under the main session that started it', async () => {
    store.now = NOW;
    const st = state([
      account({ id: 'main', label: 'Main' }),
      account({ id: 'x', label: 'Second' }),
      account({ id: 'c', label: 'Codex', provider: 'codex' }),
    ]);
    st.sessions = [
      session({ id: 'm1', accountId: 'main', state: 'working', name: 'Overview design', startedAt: NOW - 50 * 60_000 }),
      session({ id: 'm2', accountId: 'main', state: 'idle', name: 'Status skill', startedAt: NOW - 40 * 60_000 }),
      session({ id: 'k1', accountId: 'x', state: 'working', name: 'Admin', parentId: 'm1' }),
      session({ id: 'k2', accountId: 'c', provider: 'codex', state: 'waiting', name: 'Review', parentId: 'm1' }),
      session({ id: 'o', accountId: 'x', state: 'idle', name: 'Orphan', parentId: 'gone' }),
    ];
    store.state = st;
    const page = mount(OverviewPage);
    const names = () => sessionRows(page).map((r) => r.findAll('[role=cell]')[1]!);
    // The parent keeps its place; its waiting child comes first under it; the orphan stands at the top level.
    expect(names().map((c) => c.text())).toEqual([
      'Overview design',
      'Review. Started by Overview design',
      'Admin. Started by Overview design',
      'Status skill',
      'Orphan',
    ]);
    expect(names()[1]!.find('.sr-only').text()).toBe('. Started by Overview design');
    expect(names()[0]!.findAll('[aria-hidden=true]')).toHaveLength(0);
    for (const k of [1, 2]) expect(names()[k]!.findAll('[aria-hidden=true]')).toHaveLength(1);
    expect(page.get('[aria-label="Sessions shown by state"]').text()).toContain('1 waiting for you');
    // Filtered to Second: its child stays under its parent, which shows faded, only for it.
    await page.get('button[role=checkbox][aria-label$="of Second"]').trigger('click');
    expect(names().map((c) => c.text())).toEqual([
      'Overview design. Shown only for the sessions under it',
      'Admin. Started by Overview design',
      'Orphan',
    ]);
    expect(sessionRows(page).map((r) => r.classes().includes('icons-faded'))).toEqual([true, false, false]);
    expect(page.get('[aria-label="Sessions shown by state"]').text().replace(/\s+/g, ' ')).toBe(
      '1 working 0 waiting for you 1 idle',
    );
  });

  it('shows the faded parent of a checked account that only has child sessions, and does not count it', async () => {
    store.now = NOW;
    const st = state([account({ id: 'main', label: 'Main' }), account({ id: 'c', label: 'Codex', provider: 'codex' })]);
    st.sessions = [
      session({
        id: 'm1',
        accountId: 'main',
        state: 'waiting',
        doing: 'Asks to run a command',
        project: 'warden',
        name: 'Overview design',
        subagents: [{ type: 'Explore' }],
      }),
      session({
        id: 'm0',
        accountId: 'main',
        state: 'done',
        doing: 'Ended',
        name: 'Long gone',
        endedAt: NOW - 60 * 60_000,
      }),
      session({ id: 'k1', accountId: 'c', provider: 'codex', state: 'working', name: 'Review', parentId: 'm1' }),
      session({ id: 'k2', accountId: 'c', provider: 'codex', state: 'idle', name: 'Old work', parentId: 'm0' }),
      session({ id: 'm2', accountId: 'main', state: 'idle', name: 'Status skill' }),
    ];
    store.state = st;
    const page = mount(OverviewPage);
    const names = () => sessionRows(page).map((r) => r.findAll('[role=cell]')[1]!.text());
    // The ended parent is past its 10 minutes (the core no longer keeps it): its child stands at the top level,
    // filtered or not.
    expect(names()).not.toContain('Long gone');
    expect(names()).toContain('Old work');

    await page.get('button[role=checkbox][aria-label$="of Codex"]').trigger('click');
    expect(names()).toEqual([
      'Overview design. Shown only for the sessions under it',
      'Review. Started by Overview design',
      'Old work',
    ]);
    const [parent, child] = sessionRows(page);
    // Faded: no amber, its text tertiary, its icons dimmed; the provider mark keeps its colour (3:1).
    expect(parent!.classes()).toEqual(expect.arrayContaining(['icons-faded', 'text-tertiary']));
    expect(parent!.classes()).not.toContain('bg-attn-bg');
    const cells = parent!.findAll('[role=cell]');
    expect(cells[2]!.classes()).toContain('text-tertiary');
    expect(cells[2]!.classes()).not.toContain('text-attn');
    expect(cells[3]!.classes()).toContain('text-tertiary');
    expect(cells[4]!.find('img').classes()).not.toContain('opacity-55');
    expect(child!.classes()).not.toContain('icons-faded');
    expect(child!.find('[role=cell]').attributes('aria-label')).toBe('Working');
    // Only the Codex sessions count.
    expect(page.get('[aria-label="Sessions shown by state"]').text().replace(/\s+/g, ' ')).toBe(
      '1 working 0 waiting for you 1 idle',
    );
    expect(page.text()).not.toContain('No sessions for the checked accounts');
  });

  it('shows the end of each session id in its own column, so time-ordered ids started together differ', () => {
    store.now = NOW;
    const st = state([account({ id: 'main', label: 'Main' })]);
    // Codex's ids are UUID v7: sessions started within about a minute share their first 8 characters.
    st.sessions = [
      session({ id: '019a8c3e-1f2a-7b3c-9d4e-5f6a7b8c9d0e', accountId: 'main', state: 'working', name: 'Same task' }),
      session({ id: '019a8c3e-4b5c-7d6e-8f90-a1b2c3d4e5f6', accountId: 'main', state: 'working', name: 'Same task' }),
    ];
    store.state = st;
    const page = mount(OverviewPage);
    const heads = page.findAll('[aria-labelledby=sessions-heading] [role=columnheader]').map((h) => h.text());
    expect(heads.slice(5)).toEqual(['Subagents', 'ID', 'Running for']);
    const ids = sessionRows(page).map((r) => r.findAll('[role=cell]')[6]!);
    expect(ids.map((c) => c.text())).toEqual(['7b8c9d0e', 'c3d4e5f6']);
    expect(ids[0]!.classes()).toEqual(expect.arrayContaining(['font-mono', 'text-tertiary']));
  });

  it('draws each part of the connectors once, through the row border and padding without overlap', () => {
    store.now = NOW;
    const st = state([account({ id: 'main', label: 'Main' })]);
    const at = (k: number) => NOW - (20 - k) * 60_000;
    st.sessions = [
      session({ id: 'm', accountId: 'main', state: 'working', startedAt: at(0) }),
      session({ id: 'a', accountId: 'main', state: 'working', parentId: 'm', startedAt: at(1) }),
      session({ id: 'a1', accountId: 'main', state: 'working', parentId: 'a', startedAt: at(2) }),
      session({ id: 'a2', accountId: 'main', state: 'working', parentId: 'a', startedAt: at(3) }),
      session({ id: 'b', accountId: 'main', state: 'working', parentId: 'm', startedAt: at(4) }),
      session({ id: 'b1', accountId: 'main', state: 'working', parentId: 'b', startedAt: at(5) }),
    ];
    store.state = st;
    const page = mount(OverviewPage);
    // Per row, per level: the parts drawn there.
    const parts = sessionRows(page).map((r) =>
      r
        .findAll('[role=cell]')[1]!
        .findAll(':scope > [aria-hidden=true]')
        .map((level) =>
          level
            .findAll('[data-line]')
            .map((l) => l.attributes('data-line'))
            .join('+'),
        ),
    );
    expect(parts).toEqual([[], ['rail+stub'], ['rail', 'rail+stub'], ['rail', 'elbow'], ['elbow'], ['', 'elbow']]);
    for (const level of page.findAll('[aria-labelledby=sessions-heading] [role=cell] > [aria-hidden=true]')) {
      const lines = level.findAll('[data-line]');
      // One vertical and one horizontal line at most in a level, each from one element.
      expect(lines.filter((l) => l.classes().includes('border-l'))).toHaveLength(Math.min(1, lines.length));
      expect(lines.filter((l) => l.classes().includes('border-b')).length).toBeLessThanOrEqual(1);
      for (const l of lines) {
        const c = l.classes();
        // From the top of the row's border to the bottom of its padding: the border and py-2.25 above, py-2.25 below.
        if (c.includes('border-l')) expect(c).toContain('-top-2.5');
        if (l.attributes('data-line') === 'rail') expect(c).toContain('-bottom-2.25');
        // The stub starts right of the rail, which has the pixel they share.
        if (l.attributes('data-line') === 'stub') expect(c).toContain('left-2.25');
      }
    }
    expect(sessionRows(page)[0]!.classes()).toContain('py-2.25');
  });

  it('lists sessions with waiting rows first and counts them in the legend', () => {
    const page = withSessions();
    const rows = sessionRows(page);
    expect(rows.map((r) => r.find('[role=cell]').attributes('aria-label'))).toEqual([
      'Waiting for you',
      'Working',
      'Idle',
    ]);
    expect(rows[0]!.classes()).toContain('bg-attn-bg');
    expect(rows[0]!.text()).toContain('#55402: Fix login');
    expect(rows[0]!.text()).toContain('Asks to run a command');
    expect(rows[1]!.find('button[aria-label*=subagents]').attributes('aria-label')).toBe('2 subagents: Explore, Plan');
    expect(rows[1]!.text()).toContain('30m');
    expect(page.get('[aria-label="Sessions shown by state"]').text().replace(/s+/g, ' ')).toBe(
      '1 working 1 waiting for you 1 idle',
    );
    expect(page.text()).not.toContain('Session off');
    expect(page.text()).not.toContain('Session old');
  });

  it('shows the now chip and the sparkline per account', () => {
    const page = withSessions();
    const accountRows = page.findAll('[aria-labelledby=accounts-heading] [role=row]');
    expect(accountRows[1]!.find('[aria-label^="Now:"]').attributes('aria-label')).toBe('Now: 1 working, 1 idle');
    const second = accountRows[2]!;
    expect(second.find('[aria-label^="Now:"]').classes()).toContain('bg-attn-bg');
    expect(second.find('svg').attributes('aria-label')).toBe(
      'Second, last 2 hours: up to 2 sessions working, waited for you once',
    );
    expect(second.findAll('circle')).toHaveLength(1);
  });

  it('shows a session signed in as someone else in amber and counts it for no account', () => {
    store.now = NOW;
    const st = state([
      account({
        id: 'main',
        label: 'Main',
        identity: { email: 'main@example.com', organization: null },
        identityChange: { now: { email: 'me@home.example', organization: null }, knownAccountId: null },
      }),
    ]);
    st.sessions = [
      session({ id: 'x', accountId: 'main', state: 'working', signedInAs: 'me@home.example' }),
      session({ id: 'w', accountId: 'main', state: 'idle' }),
    ];
    store.state = st;
    const page = mount(OverviewPage);
    const account0 = page.findAll('[aria-labelledby=accounts-heading] [role=row]')[1]!;
    expect(account0.text()).not.toContain('main@example.com');
    expect(account0.get('button.text-attn').text()).toBe('Signed in as me@home.example');
    expect(account0.find('[aria-label^="Now:"]').attributes('aria-label')).toBe('Now: 1 idle');
    const rows = sessionRows(page);
    expect(rows[0]!.findAll('[role=cell]')[4]!.text()).toBe('Signed in as me@home.example');
    expect(rows[1]!.findAll('[role=cell]')[4]!.text()).toBe('Main');
  });

  it('keeps every sessions row on one line: name, doing now, project and account truncate, the full text in title', () => {
    store.now = NOW;
    const st = state([account({ id: 'main', label: 'Main' }), account({ id: 'x', label: 'Second account' })]);
    st.sessions = [
      session({ id: 'm1', accountId: 'main', state: 'waiting', name: 'Overview design', project: 'warden' }),
      session({
        id: 'k1',
        accountId: 'x',
        state: 'idle',
        doing: 'Waiting for your next message',
        project: 'a-rather-long-project-name',
        name: 'A long session name that would wrap in a narrow table',
        parentId: 'm1',
      }),
      session({ id: 'w', accountId: 'main', state: 'working', signedInAs: 'someone-else@example.com' }),
    ];
    store.state = st;
    const page = mount(OverviewPage);
    const rows = sessionRows(page);
    const [, child, signedIn] = rows;
    const cells = child!.findAll('[role=cell]');
    const name = cells[1]!.get('.truncate');
    expect(name.attributes('title')).toBe('A long session name that would wrap in a narrow table');
    // Screen readers still hear who started it.
    expect(name.text()).toBe('A long session name that would wrap in a narrow table. Started by Overview design');
    expect(cells[2]!.attributes('title')).toBe('Waiting for your next message');
    expect(cells[3]!.attributes('title')).toBe('a-rather-long-project-name');
    const label = cells[4]!.get('.truncate');
    expect(label.text()).toBe('Second account');
    expect(label.attributes('title')).toBe('Second account');
    for (const el of [name, cells[2]!, cells[3]!, label]) {
      expect(el.classes()).toEqual(expect.arrayContaining(['truncate', 'min-w-0']));
      expect(el.classes()).not.toContain('break-words');
    }
    const other = signedIn!.findAll('[role=cell]')[4]!.get('.truncate');
    expect(other.classes()).toContain('text-attn');
    expect(other.attributes('title')).toBe('Signed in as someone-else@example.com');
    expect(page.html()).not.toContain('break-words');
  });

  it('filters the sessions by the checked accounts, with chips and Show all', async () => {
    const page = withSessions();
    expect(page.text()).not.toContain('Stop showing only');
    await page.findAll('[aria-labelledby=accounts-heading] [role=row]')[2]!.trigger('click');
    expect(page.get('button[role=checkbox][aria-checked=true]').attributes('aria-label')).toBe(
      'Show only the sessions of Second',
    );
    expect(sessionRows(page)).toHaveLength(1);
    expect(page.get('[aria-label="Stop showing only the sessions of Second"]').text()).toBe('Second');
    await page.get('button:not([role])').trigger('click');
    const showAll = page.findAll('button').find((b) => b.text() === 'Show all');
    expect(showAll).toBeUndefined();
    expect(sessionRows(page)).toHaveLength(3);
  });

  it('sorts sessions of one state oldest first, and shows a finished one for 10 minutes, last and uncounted', () => {
    store.now = NOW;
    const st = state([account({ id: 'main', label: 'Main' })]);
    st.sessions = [
      session({ id: 'done', accountId: 'main', state: 'done', doing: 'Ended', endedAt: NOW - 5 * 60_000 }),
      session({ id: 'new', accountId: 'main', state: 'working', startedAt: NOW - 5 * 60_000 }),
      session({ id: 'old', accountId: 'main', state: 'working', startedAt: NOW - 50 * 60_000 }),
      session({ id: 'wait', accountId: 'main', state: 'waiting', startedAt: NOW - 60_000 }),
    ];
    store.state = st;
    const page = mount(OverviewPage);
    expect(sessionRows(page).map((r) => r.findAll('[role=cell]')[1]!.text())).toEqual([
      'Session wait',
      'Session old',
      'Session new',
      'Session done',
    ]);
    expect(page.get('[aria-label="Sessions shown by state"]').text().replace(/\s+/g, ' ')).toBe(
      '2 working 1 waiting for you 0 idle',
    );
  });

  it('does not mark a waiting session from the session list in amber', () => {
    store.now = NOW;
    const st = state([account({ id: 'main', label: 'Main' })]);
    st.sessions = [session({ id: 'l', accountId: 'main', state: 'waiting', source: 'list' })];
    store.state = st;
    expect(sessionRows(mount(OverviewPage))[0]!.classes()).not.toContain('bg-attn-bg');
  });

  it('fades the whole row of an ended session and of one whose state is not known, but not a working one', () => {
    store.now = NOW;
    const st = state([account({ id: 'main', label: 'Main' })]);
    st.sessions = [
      session({ id: 'work', accountId: 'main', state: 'working', doing: 'Editing files' }),
      session({ id: 'listed', accountId: 'main', state: 'waiting', doing: 'Seen in the list', source: 'list' }),
      session({
        id: 'ended',
        accountId: 'main',
        state: 'done',
        doing: 'Ended',
        endedAt: NOW - 60_000,
        signedInAs: 'sam@example.com',
      }),
    ];
    store.state = st;
    const page = mount(OverviewPage);
    const [work, listed, ended] = sessionRows(page);
    expect(work!.findAll('[role=cell]')[1]!.text()).toBe('Session work');
    expect(work!.classes()).not.toContain('icons-faded');
    expect(work!.classes()).not.toContain('text-tertiary');
    expect(work!.findAll('[role=cell]')[2]!.classes()).toContain('text-secondary');
    for (const [row, label] of [
      [listed!, 'State not known, no status hook'],
      [ended!, 'Done'],
    ] as const) {
      expect(row.classes()).toEqual(expect.arrayContaining(['icons-faded', 'text-tertiary']));
      expect(row.classes()).not.toContain('bg-attn-bg');
      expect(row.findAll('[role=cell]')[0]!.attributes('aria-label')).toBe(label);
      expect(row.text()).not.toContain('Shown only for the sessions under it');
      // No amber anywhere in it: the doing text and a changed sign-in are tertiary too.
      expect(row.findAll('.text-attn')).toHaveLength(0);
      expect(row.findAll('[role=cell]')[2]!.classes()).toContain('text-tertiary');
    }
    expect(ended!.text()).toContain('Signed in as sam@example.com');
    // They still count as before: the listed one as not known (not shown in the counts), the ended one not at all.
    expect(page.get('[aria-label="Sessions shown by state"]').text().replace(/\s+/g, ' ')).toBe(
      '1 working 0 waiting for you 0 idle',
    );
  });

  it('says when there are no sessions at all', () => {
    store.now = NOW;
    store.state = state([account({ id: 'main', label: 'Main' })]);
    expect(mount(OverviewPage).text()).toContain('No sessions right now');
  });

  describe('the ended and not-known checkbox', () => {
    function quietState(show = true) {
      store.now = NOW;
      const st = state([account({ id: 'main', label: 'Main' }), account({ id: 'x', label: 'Second' })]);
      st.settings = { ...st.settings, showQuietSessions: show };
      st.sessions = [
        session({ id: 'm1', accountId: 'main', state: 'done', name: 'Ended parent', endedAt: NOW - 60_000 }),
        session({ id: 'k1', accountId: 'x', state: 'working', name: 'Child', parentId: 'm1' }),
        session({ id: 'm2', accountId: 'main', state: 'idle', name: 'Listed', source: 'list' }),
        session({ id: 'm3', accountId: 'main', state: 'idle', name: 'Idle' }),
        session({ id: 'x2', accountId: 'x', state: 'done', name: 'Ended alone', endedAt: NOW - 60_000 }),
      ];
      return st;
    }
    const toggle = (page: ReturnType<typeof mount>) => page.find('[data-quiet-toggle]');
    const box = (page: ReturnType<typeof mount>) => toggle(page).get('[role=checkbox]');
    const names = (page: ReturnType<typeof mount>) => sessionRows(page).map((r) => r.findAll('[role=cell]')[1]!.text());

    it('shows only while there is such a session, with how many after the account filter', async () => {
      store.state = quietState();
      const page = mount(OverviewPage);
      expect(toggle(page).text().replace(/\s+/g, ' ')).toBe('Ended and not known 3');
      expect(box(page).attributes('aria-checked')).toBe('true');
      // A labelled checkbox at the right end, after the counts.
      const right = page.get('#sessions-heading').element.parentElement!.lastElementChild!;
      expect(right.firstElementChild!.getAttribute('aria-label')).toBe('Sessions shown by state');
      expect(right.lastElementChild).toBe(toggle(page).element);
      expect(toggle(page).get('label').attributes('for')).toBe(box(page).attributes('id'));
      await page.get('button[role=checkbox][aria-label$="of Second"]').trigger('click');
      expect(toggle(page).text().replace(/\s+/g, ' ')).toBe('Ended and not known 1');

      const st = quietState();
      st.sessions = st.sessions.filter((s) => s.state !== 'done' && s.source !== 'list');
      store.state = st;
      expect(toggle(mount(OverviewPage)).exists()).toBe(false);
    });

    it('hides ended and not-known rows while off, keeping a faded parent of a shown one, and the counts', () => {
      store.state = quietState(false);
      const page = mount(OverviewPage);
      expect(box(page).attributes('aria-checked')).toBe('false');
      expect(names(page)).toEqual([
        'Idle',
        'Ended parent. Shown only for the sessions under it',
        'Child. Started by Ended parent',
      ]);
      expect(sessionRows(page).map((r) => r.classes().includes('icons-faded'))).toEqual([false, true, false]);
      expect(page.get('[aria-label="Sessions shown by state"]').text().replace(/\s+/g, ' ')).toBe(
        '1 working 0 waiting for you 1 idle',
      );

      store.state = quietState(true);
      const shown = mount(OverviewPage);
      expect(names(shown)).toHaveLength(5);
      expect(names(shown)).toContain('Ended parent');
      expect(shown.get('[aria-label="Sessions shown by state"]').text().replace(/\s+/g, ' ')).toBe(
        '1 working 0 waiting for you 1 idle',
      );
    });

    it('says when only hidden sessions are left, or the filtered message while accounts are checked', async () => {
      const st = quietState(false);
      st.sessions = st.sessions.filter((s) => s.state === 'done' || s.source === 'list');
      store.state = st;
      const page = mount(OverviewPage);
      expect(sessionRows(page)).toHaveLength(1);
      expect(page.text()).toContain('Only ended and not-known sessions, hidden');
      expect(page.text()).not.toContain('No sessions right now');
      await page.get('button[role=checkbox][aria-label$="of Second"]').trigger('click');
      expect(page.text()).toContain('No sessions for the checked accounts');
    });

    it('saves the choice when pressed', async () => {
      store.state = quietState();
      const calls = fakeServer(() => ({
        ...store.state!,
        settings: { ...store.state!.settings, showQuietSessions: false },
      }));
      const page = mount(OverviewPage);
      await box(page).trigger('click');
      await flushPromises();
      expect(calls).toEqual([{ method: 'PUT', path: '/api/settings', body: { showQuietSessions: false } }]);
      expect(box(page).attributes('aria-checked')).toBe('false');
      expect(names(page)).not.toContain('Ended alone');
    });
  });

  it('filters by several accounts, unchecks one from its box, and Show all clears the rest', async () => {
    const page = withSessions();
    const accountRows = () => page.findAll('[aria-labelledby=accounts-heading] [role=row]');
    await accountRows()[1]!.trigger('click');
    await accountRows()[2]!.trigger('click');
    expect(page.findAll('button[role=checkbox][aria-checked=true]')).toHaveLength(2);
    expect(sessionRows(page)).toHaveLength(3);
    // The box sits inside the row: one click on it toggles once.
    await page.get('[aria-label="Show only the sessions of Main"]').trigger('click');
    expect(page.get('[aria-label="Show only the sessions of Main"]').attributes('aria-checked')).toBe('false');
    expect(sessionRows(page)).toHaveLength(1);
    expect(accountRows()[2]!.classes()).toContain('bg-selected-bg');
    expect(accountRows()[1]!.classes()).not.toContain('bg-selected-bg');

    await page
      .findAll('button')
      .find((b) => b.text() === 'Show all')!
      .trigger('click');
    expect(page.findAll('button[role=checkbox][aria-checked=true]')).toHaveLength(0);
    expect(page.text()).not.toContain('Stop showing only');
    expect(sessionRows(page)).toHaveLength(3);
  });

  it('says when the checked accounts have no sessions', async () => {
    store.now = NOW;
    const st = state([account({ id: 'main', label: 'Main' }), account({ id: 'x', label: 'Second' })]);
    st.sessions = [session({ id: 'w1', accountId: 'main', state: 'working' })];
    store.state = st;
    const page = mount(OverviewPage);
    await page.findAll('[aria-labelledby=accounts-heading] [role=row]')[2]!.trigger('click');
    expect(page.text()).toContain('No sessions for the checked accounts');
  });

  it('forgets a checked account once it is switched off, showing every session again', async () => {
    const page = withSessions();
    await page.findAll('[aria-labelledby=accounts-heading] [role=row]')[2]!.trigger('click');
    expect(sessionRows(page)).toHaveLength(1);
    const st = store.state!;
    store.state = { ...st, accounts: st.accounts.map((a) => (a.id === 'x' ? { ...a, enabled: false } : a)) };
    await page.vm.$nextTick();
    expect(page.text()).not.toContain('Stop showing only');
    expect(sessionRows(page).map((r) => r.find('[role=cell]').attributes('aria-label'))).toEqual(['Working', 'Idle']);
  });

  it('lists the limit resets of the accounts in use beside them, and nothing while there are none', () => {
    store.now = NOW;
    store.state = state([
      account({ id: 'main', label: 'Main' }),
      account({ id: 'c', label: 'Codex', provider: 'codex' }),
    ]);
    expect(mount(OverviewPage).find('[aria-labelledby=resets-heading]').exists()).toBe(false);

    const DAY = 24 * HOUR;
    store.state = state([
      account({ id: 'main', label: 'Main' }),
      account({
        id: 'c',
        label: 'Codex',
        provider: 'codex',
        resets: [
          { id: null, title: 'Full reset (Weekly + 5 hr)', expiresAt: NOW + 9 * DAY },
          { id: null, title: 'Expired', expiresAt: NOW - HOUR },
        ],
      }),
      account({
        id: 'p',
        label: 'Personal',
        provider: 'codex',
        enabled: false,
        resets: [{ id: null, title: 'Off', expiresAt: null }],
      }),
    ]);
    const list = mount(OverviewPage).get('[aria-labelledby=resets-heading]');
    expect(list.get('h2').text()).toBe('Limit resets');
    const items = list.findAll('li');
    expect(items).toHaveLength(1);
    expect(items[0]!.text()).toContain('Codex');
    expect(items[0]!.text()).toContain('Full reset (Weekly + 5 hr)');
    expect(items[0]!.text()).toContain('Expires 18 Nov, 10:00');
    expect(items[0]!.find('[data-expires-soon]').exists()).toBe(false);
  });

  it("lists main's Claude resets one per row, soonest first, the one inside the warning days in amber", () => {
    store.now = NOW;
    const DAY = 24 * HOUR;
    const st = state([
      account({
        id: 'main',
        label: 'Main',
        resets: [
          { id: null, title: 'Full reset', expiresAt: NOW + 2 * DAY },
          { id: null, title: 'Full reset', expiresAt: NOW + 2 * DAY },
          { id: null, title: '5-hour reset', expiresAt: NOW + 9 * DAY },
        ],
      }),
    ]);
    store.state = st;
    const list = () => mount(OverviewPage).get('[aria-labelledby=resets-heading]');
    let items = list().findAll('li');
    expect(items.map((i) => i.text().includes('Full reset'))).toEqual([true, true, false]);
    const soon = items[0]!.get('[data-expires-soon]');
    expect(soon.classes()).toContain('text-attn');
    expect(soon.text()).toMatch(/^Expires in 2 days, \d+ \w+, \d\d:\d\d$/);
    expect(soon.find('svg').attributes('aria-hidden')).toBe('true');
    expect(soon.find('svg').classes()).toContain('text-attn-icon');
    expect(items[2]!.find('[data-expires-soon]').exists()).toBe(false);

    // A shorter warning set in Settings: none is amber.
    store.state = { ...st, settings: { ...st.settings, resetWarningDays: 1 } };
    items = list().findAll('li');
    expect(items.some((i) => i.find('[data-expires-soon]').exists())).toBe(false);
  });

  describe('using a limit reset', () => {
    const DAY = 24 * HOUR;
    const SESSION: Reset = { id: 'grant-s', title: '5-hour reset', expiresAt: NOW + 3 * DAY };
    const FULL: Reset = { id: 'grant-full', title: 'Full reset', expiresAt: NOW + 2 * DAY };
    const CODEX: Reset = { id: 'credit-1', title: 'Full reset (Weekly + 5 hr)', expiresAt: NOW + 5 * DAY };
    /** The box every element reports while motion is stood in for (happy-dom lays nothing out). */
    const BOX = { left: 10, top: 20, width: 300, height: 60, right: 310, bottom: 80 };

    function withResets(main: Reset[] = [FULL, FULL, SESSION, { id: null, title: 'Old', expiresAt: NOW + 4 * DAY }]) {
      store.now = NOW;
      return state([
        account({ id: 'main', label: 'Main', resets: main }),
        account({ id: 'c', label: 'Codex', provider: 'codex', resets: [CODEX] }),
      ]);
    }

    const restore: (() => void)[] = [];
    afterEach(() => {
      restore.splice(0).forEach((undo) => undo());
      document.body.innerHTML = '';
    });

    /**
     * Stands in for the Web Animations API and the row's box: each animation is recorded and finishes at once, or with
     * `finished` (one for all, or a new one for each animation).
     */
    function stubMotion(
      reduce: boolean,
      finished: Promise<void> | (() => Promise<void>) = Promise.resolve(),
    ): Keyframe[][] {
      const animations: Keyframe[][] = [];
      const proto = HTMLElement.prototype as unknown as Record<string, unknown>;
      for (const [name, value] of Object.entries({
        animate(keyframes: Keyframe[]) {
          animations.push(keyframes);
          return { finished: typeof finished === 'function' ? finished() : finished, cancel() {} };
        },
        getBoundingClientRect: () => BOX,
      })) {
        const had = Object.getOwnPropertyDescriptor(proto, name);
        Object.defineProperty(proto, name, { value, configurable: true, writable: true });
        restore.push(() => (had ? Object.defineProperty(proto, name, had) : delete proto[name]));
      }
      vi.stubGlobal('matchMedia', (query: string) => ({
        matches: reduce && query.includes('reduce'),
        media: query,
        onchange: null,
        addEventListener() {},
        removeEventListener() {},
        addListener() {},
        removeListener() {},
        dispatchEvent: () => true,
      }));
      return animations;
    }

    const dialog = () => document.querySelector<HTMLElement>('[role=dialog]');
    const useButtons = (page: ReturnType<typeof mount>) => page.findAll('[data-use-reset]');

    it('gives each usable reset a small Use button named for the reset and its account', () => {
      store.state = withResets();
      const page = mount(OverviewPage);
      expect(useButtons(page).map((b) => [b.text(), b.attributes('aria-label')])).toEqual([
        ['Use', 'Use Full reset on Main'],
        ['Use', 'Use Full reset on Main'],
        ['Use', 'Use 5-hour reset on Main'],
        ['Use', 'Use Full reset (Weekly + 5 hr) on Codex'],
      ]);
      // An outline button, small, on the row; a reset without an id (read before ids were kept) has none.
      const button = useButtons(page)[0]!;
      expect(button.classes()).toEqual(expect.arrayContaining(['h-7', 'inset-ring-1', 'text-12']));
      const old = page.findAll('li').find((li) => li.text().includes('Old'))!;
      expect(old.find('[data-use-reset]').exists()).toBe(false);
    });

    it('asks first, saying what each kind of reset does and that it cannot be undone', async () => {
      store.state = withResets();
      const page = mount(OverviewPage, { attachTo: document.body });
      for (const [label, title, line] of [
        [
          'Use Full reset on Main',
          'Use a Full reset on Main?',
          "It refills its 5-hour and weekly limits now. This can't be undone.",
        ],
        [
          'Use 5-hour reset on Main',
          'Use a 5-hour reset on Main?',
          "It refills its 5-hour limit now. This can't be undone.",
        ],
        [
          'Use Full reset (Weekly + 5 hr) on Codex',
          'Use a Full reset (Weekly + 5 hr) on Codex?',
          "It refills its 5-hour and weekly limits now. This can't be undone.",
        ],
      ] as const) {
        await page.get(`[aria-label="${label}"]`).trigger('click');
        await flushPromises();
        const d = dialog()!;
        expect(d.querySelector('h2')!.textContent!.trim()).toBe(title);
        expect(document.getElementById(d.getAttribute('aria-labelledby')!)?.textContent).toBe(title);
        expect(d.querySelector('h2 + span')!.textContent!.trim()).toBe(line);
        // "Use reset" first, then Cancel, which closes it without sending anything.
        expect([...d.querySelectorAll('button')].map((b) => b.textContent!.trim())).toEqual(['Use reset', 'Cancel']);
        [...d.querySelectorAll('button')][1]!.click();
        await flushPromises();
        expect(dialog()).toBeNull();
      }
    });

    it('shows busy while it sends; a refusal stays in the modal and the row stays', async () => {
      store.state = withResets();
      let answer: (r: Response) => void = () => {};
      const calls: { path: string; body: unknown }[] = [];
      vi.stubGlobal('fetch', (path: string, init: RequestInit) => {
        calls.push({ path, body: JSON.parse(String(init.body)) });
        return new Promise<Response>((r) => (answer = r));
      });
      const page = mount(OverviewPage, { attachTo: document.body });
      await page.get('[aria-label="Use 5-hour reset on Main"]').trigger('click');
      await flushPromises();
      const submit = dialog()!.querySelector<HTMLButtonElement>('button[type=submit]')!;
      submit.click();
      await flushPromises();
      expect(calls).toEqual([{ path: '/api/accounts/main/resets/use', body: { resetId: 'grant-s' } }]);
      expect(submit.getAttribute('aria-busy')).toBe('true');
      expect(submit.querySelector('svg')!.classList).toContain('animate-spin-slow');
      // Neither Escape nor Cancel closes it while it sends.
      dialog()!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
      await flushPromises();
      expect(dialog()).not.toBeNull();
      expect(dialog()!.querySelectorAll('button')[1]!.disabled).toBe(true);

      answer(
        new Response(JSON.stringify({ error: 'This reset was already used.' }), {
          status: 409,
          headers: { 'Content-Type': 'application/json' },
        }),
      );
      await flushPromises();
      expect(dialog()).not.toBeNull();
      const error = dialog()!.querySelector('[role=alert]')!;
      expect(error.textContent).toBe('This reset was already used.');
      expect(error.classList).toContain('text-attn');
      expect(submit.getAttribute('aria-busy')).toBeNull();
      expect(useButtons(page)).toHaveLength(4);
    });

    it('once used, the modal closes and the row turns to dust, then goes', async () => {
      const animations = stubMotion(false);
      store.state = withResets();
      const after = withResets([FULL, FULL, { id: null, title: 'Old', expiresAt: NOW + 4 * DAY }]);
      const calls = fakeServer(() => after);
      const page = mount(OverviewPage, { attachTo: document.body });
      await page.get('[aria-label="Use 5-hour reset on Main"]').trigger('click');
      await flushPromises();
      dialog()!.querySelector<HTMLButtonElement>('button[type=submit]')!.click();
      await flushPromises();
      await flushPromises();
      expect(calls.map((c) => [c.method, c.path, c.body])).toEqual([
        ['POST', '/api/accounts/main/resets/use', { resetId: 'grant-s' }],
      ]);
      expect(dialog()).toBeNull();
      // Many fragments drift up and to the side and fade, then the gap closes.
      const drifts = animations.filter((k) => String(k[k.length - 1]!.transform).startsWith('translate('));
      expect(drifts.length).toBeGreaterThan(50);
      for (const k of drifts) {
        const end = k[k.length - 1]!;
        expect(end.opacity).toBe(0);
        const [dx, dy] = /translate\((-?[\d.]+)px, (-?[\d.]+)px\)/.exec(String(end.transform))!.slice(1).map(Number);
        expect(dx).toBeGreaterThan(0);
        expect(dy).toBeLessThan(0);
      }
      // The row's own height (the stand-in box's) down to nothing.
      const height = `${BOX.height}px`;
      expect(animations[animations.length - 1]!.map((k) => k.height)).toEqual([height, '0']);
      // The fragments are gone with it.
      expect(document.querySelectorAll('[aria-hidden=true].fixed')).toHaveLength(0);
      expect(page.findAll('li').map((li) => li.text().includes('5-hour reset'))).not.toContain(true);
      expect(useButtons(page)).toHaveLength(3);
      expect(page.get('[role=status]').text()).toBe('5-hour reset used on Main.');
    });

    it('with reduced motion, the row only fades out', async () => {
      const animations = stubMotion(true);
      store.state = withResets();
      fakeServer(() => withResets([FULL, SESSION, { id: null, title: 'Old', expiresAt: NOW + 4 * DAY }]));
      const page = mount(OverviewPage, { attachTo: document.body });
      await page.findAll('[aria-label="Use Full reset on Main"]')[1]!.trigger('click');
      await flushPromises();
      dialog()!.querySelector<HTMLButtonElement>('button[type=submit]')!.click();
      await flushPromises();
      await flushPromises();
      expect(animations).toEqual([[{ opacity: 1 }, { opacity: 0 }]]);
      // One of the grant's two full resets is left.
      expect(page.findAll('[aria-label="Use Full reset on Main"]')).toHaveLength(1);
      expect(useButtons(page)).toHaveLength(3);
    });
    /** Opens the confirmation from the Use button named `label` with the keyboard's focus on it, and confirms. */
    async function useWithKeyboard(page: ReturnType<typeof mount>, label: string, nth = 0): Promise<void> {
      const button = page.findAll(`[aria-label="${label}"]`)[nth]!;
      (button.element as HTMLElement).focus();
      await button.trigger('click');
      await flushPromises();
      dialog()!.querySelector<HTMLButtonElement>('button[type=submit]')!.click();
      await flushPromises();
      await flushPromises();
    }

    it('takes the answer at once, and an update during the animation is not undone when it ends', async () => {
      let finish: () => void = () => {};
      stubMotion(true, new Promise<void>((r) => (finish = r)));
      store.state = withResets();
      const answer = withResets([FULL, FULL, { id: null, title: 'Old', expiresAt: NOW + 4 * DAY }]);
      answer.revision = 2;
      fakeServer(() => answer);
      const page = mount(OverviewPage, { attachTo: document.body });
      await useWithKeyboard(page, 'Use 5-hour reset on Main');
      // The answer is the state now; only the rows on screen wait for the animation.
      expect(store.state?.revision).toBe(2);
      expect(useButtons(page)).toHaveLength(4);

      // A newer state arrives while the row is still going: Codex's reset was used elsewhere.
      const newer = withResets([FULL, FULL, { id: null, title: 'Old', expiresAt: NOW + 4 * DAY }]);
      newer.accounts = newer.accounts.map((a) => (a.id === 'c' ? { ...a, resets: [] } : a));
      newer.revision = 3;
      store.state = newer;
      await flushPromises();
      expect(useButtons(page)).toHaveLength(4);

      finish();
      await flushPromises();
      await flushPromises();
      expect(store.state?.revision).toBe(3);
      expect(useButtons(page).map((b) => b.attributes('aria-label'))).toEqual([
        'Use Full reset on Main',
        'Use Full reset on Main',
      ]);
    });

    it('never takes an answer older than the state already shown', async () => {
      stubMotion(true);
      store.state = withResets();
      store.state.revision = 5;
      const answer = withResets([FULL, FULL]);
      answer.revision = 4;
      fakeServer(() => answer);
      const page = mount(OverviewPage, { attachTo: document.body });
      await useWithKeyboard(page, 'Use 5-hour reset on Main');
      expect(store.state?.revision).toBe(5);
    });

    it('gives focus to the Use button of a reset that stays, and to the heading once none does', async () => {
      stubMotion(false);
      store.state = withResets([SESSION]);
      const page = mount(OverviewPage, { attachTo: document.body });
      // The next row's Use button takes focus as the modal closes, and keeps it after the row has gone.
      fakeServer(() => withResets([]));
      await useWithKeyboard(page, 'Use 5-hour reset on Main');
      await flushPromises();
      expect(document.activeElement?.getAttribute('aria-label')).toBe('Use Full reset (Weekly + 5 hr) on Codex');

      // The last one: focus goes to the Overview's heading, which shows it.
      const none = withResets([]);
      none.accounts = none.accounts.map((a) => ({ ...a, resets: [] }));
      fakeServer(() => none);
      await useWithKeyboard(page, 'Use Full reset (Weekly + 5 hr) on Codex');
      await flushPromises();
      expect(page.find('[aria-labelledby=resets-heading]').exists()).toBe(false);
      const h1 = page.get('h1').element;
      expect(document.activeElement).toBe(h1);
      expect(h1.getAttribute('tabindex')).toBe('-1');
      expect(h1.classList).toContain('focus-visible:outline-indicator');
    });

    it('leaves focus where the user moved it while the row was going', async () => {
      let finish: () => void = () => {};
      stubMotion(true, new Promise<void>((r) => (finish = r)));
      store.state = withResets([FULL, SESSION]);
      fakeServer(() => withResets([SESSION]));
      const page = mount(OverviewPage, { attachTo: document.body });
      await useWithKeyboard(page, 'Use Full reset on Main');
      expect(document.activeElement?.getAttribute('aria-label')).toBe('Use 5-hour reset on Main');
      const elsewhere = page.get('[aria-label="Use Full reset (Weekly + 5 hr) on Codex"]').element as HTMLElement;
      elsewhere.focus();
      finish();
      await flushPromises();
      await flushPromises();
      expect(document.activeElement).toBe(elsewhere);
    });

    it('never gives focus to a row still gone while another use is under way', async () => {
      // Each animation, and each request, finishes when the test says.
      const finish: (() => void)[] = [];
      stubMotion(true, () => new Promise<void>((r) => finish.push(r)));
      const answer: ((s: ReturnType<typeof withResets>) => void)[] = [];
      const sent: unknown[] = [];
      vi.stubGlobal('fetch', (path: string, init: RequestInit) => {
        sent.push([path, JSON.parse(String(init.body))]);
        return new Promise<Response>((r) =>
          answer.push((s) => r(new Response(JSON.stringify(s), { headers: { 'Content-Type': 'application/json' } }))),
        );
      });
      store.state = withResets([FULL, FULL, SESSION]);
      const page = mount(OverviewPage, { attachTo: document.body });
      const confirm = async (label: string) => {
        const button = page.findAll(`[aria-label="${label}"]`)[0]!;
        (button.element as HTMLElement).focus();
        await button.trigger('click');
        await flushPromises();
        dialog()!.querySelector<HTMLButtonElement>('button[type=submit]')!.click();
        await flushPromises();
      };
      const rowOf = (key: string) => document.querySelector<HTMLElement>(`[data-reset-key="${key}"]`)!;
      const focusIsVisible = () => !document.activeElement?.closest('[data-gone]');

      // The first of the grant's two full resets is used; its row (kept by the list, which now shows the grant's one
      // full reset left in it) starts to go.
      await confirm('Use Full reset on Main');
      answer[0]!(withResets([FULL, SESSION]));
      await flushPromises();
      expect(dialog()).toBeNull();
      expect(rowOf('main:grant-full:0').dataset.gone).toBe('');
      expect(focusIsVisible()).toBe(true);

      // Before it has gone, the 5-hour reset is confirmed; while that is sent, the first row's animation ends.
      await confirm('Use 5-hour reset on Main');
      expect(sent).toHaveLength(2);
      finish[0]!();
      await flushPromises();
      expect(dialog()).not.toBeNull();
      // The list holds still for the use being sent, the first row still gone.
      expect(rowOf('main:grant-full:0').dataset.gone).toBe('');

      // Used, with Codex's reset gone too: the modal closes with focus on the heading, not on the gone row's button.
      const none = withResets([FULL]);
      none.accounts = none.accounts.map((a) => (a.id === 'c' ? { ...a, resets: [] } : a));
      answer[1]!(none);
      await flushPromises();
      expect(dialog()).toBeNull();
      expect(document.activeElement).toBe(page.get('h1').element);

      // Once the 5-hour reset's row has gone, the list shows the state, the kept row back as it was and focus on its Use.
      finish[1]!();
      await flushPromises();
      await flushPromises();
      expect(useButtons(page).map((b) => b.attributes('aria-label'))).toEqual(['Use Full reset on Main']);
      expect(document.querySelector('[data-gone]')).toBeNull();
      expect(document.activeElement).toBe(useButtons(page)[0]!.element);
    });
  });
});
