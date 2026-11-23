import { enableAutoUnmount, flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import MoreMenu, { type MenuItem } from '../components/MoreMenu.vue';
import WSlider from '../components/WSlider.vue';
import { store } from '../store';
import { fakeServer } from '../test/api';
import { account, state } from '../test/state';
import AccountsSection from './AccountsSection.vue';

enableAutoUnmount(afterEach);

afterEach(() => {
  store.state = null;
  vi.unstubAllGlobals();
});

function setup(answer?: () => ReturnType<typeof state> | { status: number; error: string }) {
  store.state = state([
    account({ id: 'main', label: 'Main', identity: { email: 'main@example.com', organization: 'Example' } }),
    account({ id: 'a', label: 'Alt', identity: { email: 'alt@example.com', organization: null } }),
    account({
      id: 'b',
      label: 'Spare',
      provider: 'codex',
      identity: { email: 'spare@example.com', organization: null },
      identityChange: { now: { email: 'other@example.com', organization: null }, knownAccountId: null },
    }),
    account({ id: 'p', label: 'Personal', enabled: false, hooks: 'notInstalled' }),
  ]);
  const calls = fakeServer(answer ?? (() => store.state!));
  const page = mount(AccountsSection, { attachTo: document.body });
  return { page, calls };
}

const rowsOf = (page: VueWrapper) => page.findAll('[role=row]').slice(1);
const button = (w: { findAll: VueWrapper['findAll'] }, text: string) =>
  w.findAll('button').find((b) => b.text() === text);
const labels = (items: MenuItem[]) => items.map((i) => i.label);

describe('AccountsSection', () => {
  it('shows main first with no switch, then each account with its switch, every one with its email', () => {
    const { page } = setup();
    const rows = rowsOf(page);
    expect(rows).toHaveLength(4);
    expect(rows[0]!.find('[role=switch]').exists()).toBe(false);
    expect(rows[0]!.text()).toContain('Claude app, main@example.com, Example');
    expect(rows[0]!.text()).not.toContain('Keep for my own work');
    expect(rows[1]!.text()).toContain('Claude, alt@example.com');
    // A changed sign-in takes the email's place.
    expect(rows[2]!.text()).toContain('SpareCodex');
    expect(rows[2]!.text()).not.toContain('spare@example.com,');
    expect(rows[1]!.get('[role=switch]').attributes('aria-label')).toBe('Stop using Alt');
    expect(rows[3]!.get('[role=switch]').attributes('aria-label')).toBe('Use Personal');
    expect(rows[3]!.text()).toContain('Not used');
    expect(rows[2]!.text()).not.toContain('Not used');
  });

  it('shows each folder in a tooltip, not as a title', () => {
    const { page } = setup();
    const tips = page.findAllComponents({ name: 'WTooltip' }).map((w) => w.props('text'));
    expect(tips).toContain('%USERPROFILE%\\.claude-a');
    expect(page.find('[title*=".claude"]').exists()).toBe(false);
  });

  it('saves the reserve once per drag', async () => {
    const { page, calls } = setup();
    const slider = page.getComponent(WSlider);
    expect(slider.props()).toMatchObject({ min: 0, max: 50, step: 5, labelledby: 'reserve-label' });
    expect(page.text()).toContain(
      "The share of Main's week kept for your own work. It shrinks as Main's weekly reset nears, during your work hours when they are set.",
    );
    expect(page.get('[data-hand-off]').text()).toBe(
      'Main takes delegated work only up to 75% now: it keeps 25% of its week for you.',
    );
    // Why, from the core's figures (no weekly reading here: the whole reserve is kept).
    expect(page.get('[data-reserve-why]').text()).toBe(
      "All of your 25% is kept until Warden reads when Main's week ends.",
    );
    expect(page.text()).toContain("Main's reserve shrinks only during these hours, so evenings and weekends keep it.");
    expect(page.get('#reserve-label').text()).toBe('Keep for my own work');
    expect(page.get('[role=slider]').attributes('aria-labelledby')).toBe('reserve-label');
    slider.vm.$emit('update:modelValue', 30);
    slider.vm.$emit('update:modelValue', 35);
    await flushPromises();
    expect(calls).toEqual([]);
    expect(page.text()).toContain('35% of its week');
    // While dragging, the hand-off point follows the thumb (no weekly reading here: the full reserve counts).
    expect(page.get('[data-hand-off]').text()).toBe(
      'Main takes delegated work only up to 65% now: it keeps 35% of its week for you.',
    );
    expect(page.get('[data-reserve-why]').text()).toContain('All of your 35% is kept');
    slider.vm.$emit('commit', 35);
    await flushPromises();
    expect(calls).toEqual([{ method: 'PUT', path: '/api/settings', body: { reserve: 35 } }]);
  });

  it('puts the slider back when the server refuses the reserve', async () => {
    const { page, calls } = setup(() => ({ status: 400, error: 'The reserve must be 0 to 50 in steps of 5.' }));
    const slider = page.getComponent(WSlider);
    slider.vm.$emit('update:modelValue', 35);
    slider.vm.$emit('commit', 35);
    await flushPromises();
    expect(calls).toHaveLength(1);
    expect(page.get('[role=alert]').text()).toBe('The reserve must be 0 to 50 in steps of 5.');
    expect(page.text()).toContain('25% of its week');
    expect(page.get('[data-hand-off]').text()).toBe(
      'Main takes delegated work only up to 75% now: it keeps 25% of its week for you.',
    );
  });

  it('switches an account on and off through the server', async () => {
    const { page, calls } = setup();
    await page.findAll('[role=switch]')[2]!.trigger('click');
    await flushPromises();
    expect(calls).toEqual([{ method: 'PATCH', path: '/api/accounts/p', body: { enabled: true } }]);
  });

  it('says nothing about an account while all is well', () => {
    const { page } = setup();
    const rows = rowsOf(page);
    for (const k of [0, 1, 3]) expect(rows[k]!.find('.text-attn').exists()).toBe(false);
  });

  it('gives main a menu with Rename only, and Warden accounts Rename and Remove', () => {
    const { page } = setup();
    const all = page.findAllComponents(MoreMenu).map((m) => m.props('items') as MenuItem[]);
    expect(labels(all[0]!)).toEqual(['Rename']);
    for (const k of [1, 2, 3]) expect(labels(all[k]!)).toEqual(['Rename', 'Remove']);
    expect(all[1]!.find((i) => i.label === 'Remove')!.danger).toBe(true);
  });

  it('shows a changed sign-in in amber in place of the email, with Accept and Undo beside it', async () => {
    const { page, calls } = setup();
    const row = rowsOf(page)[2]!;
    expect(row.text()).not.toContain('spare@example.com,');
    expect(row.get('button.text-attn').text()).toBe('Signed in as other@example.com');
    expect(page.findAllComponents({ name: 'WTooltip' }).map((w) => w.props('text'))).toContain(
      'Expected spare@example.com',
    );
    await button(row, 'Accept')!.trigger('click');
    await button(row, 'Undo')!.trigger('click');
    await flushPromises();
    expect(calls).toEqual([
      { method: 'POST', path: '/api/accounts/b/identity', body: { action: 'accept' } },
      { method: 'POST', path: '/api/accounts/b/sign-in', body: undefined },
    ]);
  });

  it("tells main's changed sign-in to be fixed in the Claude app, with no button", () => {
    store.state = state([
      account({
        id: 'main',
        label: 'Main',
        identity: { email: 'main@example.com', organization: null },
        identityChange: { now: { email: 'other@example.com', organization: null }, knownAccountId: null },
      }),
    ]);
    const row = rowsOf(mount(AccountsSection, { attachTo: document.body }))[0]!;
    expect(row.text()).toContain('Signed in as other@example.com');
    expect(row.text()).toContain('Sign the Claude app back in as main@example.com');
    expect(button(row, 'Accept')).toBeUndefined();
  });

  it("offers Warden's sign-in for main's usage while it is unknown, expired or another account", async () => {
    const { page, calls } = setup();
    store.state!.main!.signIn = { state: 'signedOut', email: null };
    await flushPromises();
    expect(rowsOf(page)[0]!.text()).toContain("Main's usage unknown");
    await button(rowsOf(page)[0]!, 'Sign in')!.trigger('click');
    await flushPromises();
    store.state!.main!.signIn = { state: 'otherAccount', email: 'other@example.com' };
    await flushPromises();
    expect(rowsOf(page)[0]!.text()).toContain("Warden is signed in as other@example.com for main's usage.");
    await button(rowsOf(page)[0]!, 'Sign in again')!.trigger('click');
    await flushPromises();
    expect(calls.map((c) => c.path)).toEqual(['/api/main/sign-in', '/api/main/sign-in']);
  });

  it('offers Sign in on a signed-out account, and says so for one signed in as main', async () => {
    const { page, calls } = setup();
    store.state!.accounts[0]!.usageError = { kind: 'signedOut' };
    store.state!.accounts[2]!.sameAsMain = true;
    await flushPromises();
    const rows = rowsOf(page);
    expect(rows[1]!.text()).toContain('Signed out');
    expect(rows[3]!.text()).toContain('Same account as main');
    expect(rows[3]!.findAll('li button')).toHaveLength(0);
    await button(rows[1]!, 'Sign in')!.trigger('click');
    await flushPromises();
    expect(calls).toEqual([{ method: 'POST', path: '/api/accounts/a/sign-in', body: undefined }]);
  });

  it("shows the Claude app's problems on main, plainly", async () => {
    const { page } = setup();
    store.state!.problems = [{ kind: 'claudeAppSignedOut' }];
    await flushPromises();
    const rows = rowsOf(page);
    expect(rows[0]!.text()).toContain('The Claude app is signed out.');
    expect(rows[1]!.text()).not.toContain('Claude app is signed out');
  });

  it('shows a missing or old CLI with Scan again on the rows of its provider only', async () => {
    const { page, calls } = setup();
    store.state!.problems = [
      { kind: 'cliTooOld', provider: 'codex', name: 'codex', version: '0.1.0', needed: '1.0.0' },
    ];
    await flushPromises();
    const rows = rowsOf(page);
    expect(rows[2]!.text()).toContain('The Codex command line (codex 0.1.0) is too old; update to 1.0.0 or later.');
    expect(rows[0]!.text()).not.toContain('command line');
    await button(rows[2]!, 'Scan again')!.trigger('click');
    await flushPromises();
    expect(calls.map((c) => c.path)).toEqual(['/api/tools/scan']);
  });

  it("says how every account's hooks are in the Hooks column, with Repair where they need it", async () => {
    const { page, calls } = setup();
    const hookText = () => rowsOf(page).map((r) => r.get('[data-hooks]').text().replace('Repair', '').trim());
    // All installed but Personal, which is switched off and has none.
    expect(hookText()).toEqual(['Installed', 'Installed', 'Installed', 'Not installed']);
    expect(rowsOf(page)[0]!.find('[data-hooks] svg').exists()).toBe(true);
    // Main's cell names both hooks in its tooltip.
    expect(page.findAllComponents({ name: 'WTooltip' }).map((w) => w.props('text'))).toContain(
      'The routing hook and the status hook',
    );
    store.state!.main!.hooks = 'notInstalled';
    store.state!.accounts[0]!.hooks = 'notTrusted';
    store.state!.accounts[2]!.hooks = 'installed';
    await flushPromises();
    expect(hookText()).toEqual(['Missing', 'Not trusted', 'Installed', 'Still installed']);
    const rows = rowsOf(page);
    expect(rows[0]!.get('[data-hooks] .text-attn').text()).toBe('Missing');
    expect(button(rows[1]!, 'Repair')!.attributes('aria-label')).toBe('Repair the hooks of Alt');
    await button(rows[1]!, 'Repair')!.trigger('click');
    await button(rows[0]!, 'Repair')!.trigger('click');
    await button(rows[3]!, 'Repair')!.trigger('click');
    await flushPromises();
    expect(calls.map((c) => c.path)).toEqual([
      '/api/accounts/a/hooks/repair',
      '/api/accounts/main/hooks/repair',
      '/api/accounts/p/hooks/repair',
    ]);
    // The row's problems no longer carry the hooks.
    expect(rows[1]!.find('ul').exists()).toBe(false);
  });

  it('says hooks are off for this run on every row, with no Repair', async () => {
    const { page } = setup();
    store.state!.hooksOff = true;
    store.state!.accounts[0]!.hooks = 'notInstalled';
    await flushPromises();
    for (const r of rowsOf(page)) {
      expect(r.get('[data-hooks]').text()).toBe('Off for this run');
      expect(button(r, 'Repair')).toBeUndefined();
    }
  });

  it('switches work hours on with Monday to Friday 09:00–18:00, and off', async () => {
    const { page, calls } = setup();
    expect(page.find('[data-work-hours]').exists()).toBe(false);
    expect(page.text()).toContain("Main's reserve shrinks only during these hours, so evenings and weekends keep it.");
    await page.get('#work-hours').trigger('click');
    await flushPromises();
    expect(calls).toEqual([
      { method: 'PUT', path: '/api/settings', body: { workHours: { days: [1, 2, 3, 4, 5], from: 540, to: 1080 } } },
    ]);
    store.state!.settings.workHours = { days: [1, 2, 3, 4, 5], from: 540, to: 1080 };
    await flushPromises();
    expect(page.get('[data-work-hours]').text()).toBe('09:00–18:00');
    await page.get('#work-hours').trigger('click');
    await flushPromises();
    expect(calls.at(-1)).toEqual({ method: 'PUT', path: '/api/settings', body: { workHours: null } });
  });

  it('saves the days at once, keeps the last day, and saves the hours once per drag', async () => {
    const { page, calls } = setup();
    store.state!.settings.workHours = { days: [1, 3], from: 540, to: 1080 };
    await flushPromises();
    const chip = (name: string) => page.findAll('[role=group] button').find((b) => b.text().includes(name))!;
    expect(chip('Mon').attributes('aria-pressed')).toBe('true');
    expect(chip('Tue').attributes('aria-pressed')).toBe('false');
    await chip('Tue').trigger('click');
    await flushPromises();
    expect(calls.at(-1)!.body).toEqual({ workHours: { days: [1, 2, 3], from: 540, to: 1080 } });
    // The last ticked day cannot be unticked: it says why, and nothing is sent.
    store.state!.settings.workHours = { days: [3], from: 540, to: 1080 };
    await flushPromises();
    const sent = calls.length;
    expect(chip('Wed').attributes('aria-disabled')).toBe('true');
    await chip('Wed').trigger('click');
    await flushPromises();
    expect(calls).toHaveLength(sent);
    expect(page.findAllComponents({ name: 'WTooltip' }).map((w) => w.props('text'))).toContain('At least one day');
    // The hours: a range, saved where it is let go.
    const range = page.findAllComponents(WSlider).find((s) => s.props('range'))!;
    range.vm.$emit('update:modelValue', [480, 1080]);
    await flushPromises();
    expect(page.get('[data-work-hours]').text()).toBe('08:00–18:00');
    expect(calls).toHaveLength(sent);
    range.vm.$emit('commit', [480, 1020]);
    await flushPromises();
    expect(calls.at(-1)!.body).toEqual({ workHours: { days: [3], from: 480, to: 1020 } });
  });

  it('puts the hours back when the server refuses them', async () => {
    const { page } = setup(() => ({ status: 400, error: 'Work hours need at least one day.' }));
    store.state!.settings.workHours = { days: [1], from: 540, to: 1080 };
    await flushPromises();
    const range = page.findAllComponents(WSlider).find((s) => s.props('range'))!;
    range.vm.$emit('update:modelValue', [600, 1080]);
    range.vm.$emit('commit', [600, 1080]);
    await flushPromises();
    expect(page.get('[role=alert]').text()).toBe('Work hours need at least one day.');
    expect(page.get('[data-work-hours]').text()).toBe('09:00–18:00');
  });

  it('says so when there are no accounts', () => {
    store.state = state([]);
    expect(mount(AccountsSection).text()).toContain('No accounts yet.');
  });

  it('keeps the hours being dragged when a new state arrives, and saves the dragged range', async () => {
    const { page, calls } = setup();
    store.state!.settings.workHours = { days: [1, 2, 3, 4, 5], from: 540, to: 1080 };
    await flushPromises();
    const range = page.findAllComponents(WSlider).find((s) => s.props('range'))!;
    range.vm.$emit('update:modelValue', [480, 1080]);
    await flushPromises();
    // A session event: the whole state is replaced, the saved hours unchanged.
    store.state = {
      ...store.state!,
      settings: { ...store.state!.settings, workHours: { days: [1, 2, 3, 4, 5], from: 540, to: 1080 } },
    };
    await flushPromises();
    expect(page.get('[data-work-hours]').text()).toBe('08:00–18:00');
    range.vm.$emit('commit', [480, 1080]);
    await flushPromises();
    expect(calls.at(-1)!.body).toEqual({ workHours: { days: [1, 2, 3, 4, 5], from: 480, to: 1080 } });
  });

  it('builds quick day clicks on each other while the server has not answered yet', async () => {
    store.state = state([account({ id: 'main', label: 'Main' })]);
    store.state.settings.workHours = { days: [1], from: 540, to: 1080 };
    const bodies: unknown[] = [];
    const answers: (() => void)[] = [];
    vi.stubGlobal('fetch', (_: string, init: RequestInit = {}) => {
      bodies.push(JSON.parse(init.body as string));
      return new Promise<Response>((resolve) =>
        answers.push(() => resolve(new Response(JSON.stringify(store.state), { status: 200 }))),
      );
    });
    const page = mount(AccountsSection, { attachTo: document.body });
    const chip = (name: string) => page.findAll('[role=group] button').find((b) => b.text().includes(name))!;
    await chip('Tue').trigger('click');
    await chip('Wed').trigger('click');
    await flushPromises();
    expect(bodies).toEqual([
      { workHours: { days: [1, 2], from: 540, to: 1080 } },
      { workHours: { days: [1, 2, 3], from: 540, to: 1080 } },
    ]);
    expect(chip('Tue').attributes('aria-pressed')).toBe('true');
    expect(chip('Wed').attributes('aria-pressed')).toBe('true');
    for (const answer of answers) answer();
    await flushPromises();
  });
});
