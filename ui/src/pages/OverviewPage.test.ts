import { mount } from '@vue/test-utils';
import { afterEach, describe, expect, it } from 'vitest';
import { store } from '../store';
import type { Session } from '../api';
import { NOW, account, state } from '../test/state';
import OverviewPage from './OverviewPage.vue';

const HOUR = 3_600_000;

afterEach(() => {
  store.state = null;
  store.connected = true;
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

  it('shows the start of each session id in its own column', () => {
    store.now = NOW;
    const st = state([account({ id: 'main', label: 'Main' })]);
    st.sessions = [
      session({ id: '0a1b2c3d-4e5f-6789', accountId: 'main', state: 'working', name: 'Same task' }),
      session({ id: '9f8e7d6c-5b4a-3210', accountId: 'main', state: 'working', name: 'Same task' }),
    ];
    store.state = st;
    const page = mount(OverviewPage);
    const heads = page.findAll('[aria-labelledby=sessions-heading] [role=columnheader]').map((h) => h.text());
    expect(heads.slice(5)).toEqual(['Subagents', 'ID', 'Running for']);
    const ids = sessionRows(page).map((r) => r.findAll('[role=cell]')[6]!);
    expect(ids.map((c) => c.text())).toEqual(['0a1b2c3d', '9f8e7d6c']);
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

  it('says when there are no sessions at all', () => {
    store.now = NOW;
    store.state = state([account({ id: 'main', label: 'Main' })]);
    expect(mount(OverviewPage).text()).toContain('No sessions right now');
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
          { title: 'Full reset (Weekly + 5 hr)', expiresAt: NOW + 9 * DAY },
          { title: 'Expired', expiresAt: NOW - HOUR },
        ],
      }),
      account({
        id: 'p',
        label: 'Personal',
        provider: 'codex',
        enabled: false,
        resets: [{ title: 'Off', expiresAt: null }],
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
          { title: 'Full reset', expiresAt: NOW + 2 * DAY },
          { title: 'Full reset', expiresAt: NOW + 2 * DAY },
          { title: '5-hour reset', expiresAt: NOW + 9 * DAY },
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
});
