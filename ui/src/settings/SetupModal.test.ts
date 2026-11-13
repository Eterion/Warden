import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Candidate } from '../api';
import { store } from '../store';
import type { Call } from '../test/api';
import { account, state } from '../test/state';
import SetupModal from './SetupModal.vue';

enableAutoUnmount(afterEach);

afterEach(() => {
  store.state = null;
  vi.unstubAllGlobals();
});

function candidate(c: Partial<Candidate> & Pick<Candidate, 'folder' | 'label'>): Candidate {
  return { provider: 'claude', identity: null, signedIn: true, known: false, ...c };
}

const WORK = candidate({
  folder: '%USERPROFILE%\\.claude-work',
  label: 'Work',
  identity: { email: 'me@example.com', organization: 'Example' },
});
const OUT = candidate({ folder: '%USERPROFILE%\\.claude-old', label: 'Old', signedIn: false });

const KNOWN = candidate({ folder: '%USERPROFILE%\\.claude-known', label: 'Known', known: true });

/** First run: the found folders on GET; POSTs answer with the state set up, or fail with `refuse`. */
async function openSetup(found: Candidate[] | Error, refuse: string | null = null) {
  const calls: Call[] = [];
  vi.stubGlobal('fetch', async (path: string, init: RequestInit = {}) => {
    const method = init.method ?? 'GET';
    calls.push({ method, path, body: typeof init.body === 'string' ? JSON.parse(init.body) : undefined });
    if (method === 'GET') {
      if (found instanceof Error) throw found;
      return new Response(JSON.stringify(found), { status: 200 });
    }
    if (refuse) return new Response(JSON.stringify({ error: refuse }), { status: 400 });
    return new Response(JSON.stringify(state([])), { status: 200 });
  });
  store.state = {
    ...state([account({ id: 'main', label: 'Main', identity: { email: 'main@example.com', organization: null } })]),
    setupDone: false,
  };
  mount(SetupModal, { attachTo: document.body });
  await flushPromises();
  return calls;
}

const $ = <T extends Element = HTMLElement>(sel: string) => document.querySelector<T>(sel)!;
const button = (text: RegExp) =>
  [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => text.test(b.textContent!.trim()))!;
async function submit() {
  $('form').dispatchEvent(new Event('submit', { cancelable: true }));
  await flushPromises();
}

describe('SetupModal', () => {
  it('shows main, the Claude app’s account, with its email and that Warden installs its hooks there', async () => {
    await openSetup([WORK]);
    const main = $('section[aria-labelledby=setup-main]').textContent!;
    expect(main).toContain("The Claude app's account");
    expect(main).toContain('main@example.com');
    expect(main).toContain('Warden installs its hooks in main when you press Start.');
    expect(document.querySelector('input[type=radio]')).toBeNull();
    expect(document.body.textContent).not.toContain('Codex app');
  });

  it("offers Warden's sign-in for main's usage, or later, and calls out another account", async () => {
    const calls = await openSetup([WORK]);
    store.state!.main!.signIn = { state: 'signedOut', email: null };
    await flushPromises();
    const usage = () => $('section[aria-labelledby=setup-usage]');
    expect(usage().textContent).toContain('Or later');
    expect(button(/^Start$/).getAttribute('aria-disabled')).toBeNull();
    button(/^Sign in$/).click();
    await flushPromises();
    expect(calls.at(-1)).toEqual({ method: 'POST', path: '/api/main/sign-in', body: undefined });
    // The fake server answers with no main; put the first-run state back, now signed in as another account.
    store.state = {
      ...state([account({ id: 'main', label: 'Main', identity: { email: 'main@example.com', organization: null } })]),
      setupDone: false,
    };
    store.state.main!.signIn = { state: 'otherAccount', email: 'other@example.com' };
    await flushPromises();
    expect(usage().querySelector('.text-attn')!.textContent).toContain(
      'Warden is signed in as other@example.com, not main@example.com.',
    );
    expect(button(/^Sign in again$/)).toBeDefined();
    store.state.main!.signIn = { state: 'signedIn', email: 'main@example.com' };
    await flushPromises();
    expect(usage().textContent).toContain('Warden is signed in as main@example.com.');
    expect(usage().querySelector('button')).toBeNull();
  });

  it('ticks the folders found, shows each with its email and suggested name, and sends the ticked ones', async () => {
    const calls = await openSetup([WORK, OUT, KNOWN]);
    const boxes = document.querySelectorAll<HTMLInputElement>('input[type=checkbox]');
    expect(boxes).toHaveLength(2);
    expect(boxes[0]!.checked).toBe(true);
    expect(document.querySelector('label[for=setup-use-0]')!.textContent).toBe('Work');
    expect(document.body.textContent).toContain('me@example.com, Example');
    expect(document.querySelector('input[type=text]')).toBeNull();
    await submit();
    expect(calls.at(-1)).toEqual({
      method: 'POST',
      path: '/api/setup',
      body: { accounts: [{ provider: 'claude', folder: WORK.folder, label: 'Work' }], skip: false },
    });
  });

  it('lets a folder be unticked', async () => {
    const calls = await openSetup([WORK]);
    $<HTMLInputElement>('#setup-use-0').click();
    await submit();
    expect(calls.at(-1)!.body).toEqual({ accounts: [], skip: false });
  });

  it('allows Start without other accounts, and Skip installs nothing', async () => {
    const calls = await openSetup([OUT]);
    expect(button(/^Start$/).getAttribute('aria-disabled')).toBeNull();
    button(/^Skip for now$/).click();
    await flushPromises();
    expect(calls.at(-1)).toEqual({ method: 'POST', path: '/api/setup', body: { accounts: [], skip: true } });
  });

  it('skips on Escape', async () => {
    const calls = await openSetup([WORK]);
    document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await flushPromises();
    expect(calls.at(-1)).toEqual({ method: 'POST', path: '/api/setup', body: { accounts: [], skip: true } });
  });

  it('says when no folders were found, or when it could not look', async () => {
    await openSetup([KNOWN]);
    expect(document.body.textContent).toContain('Warden found no other Claude or Codex account folders.');
    await openSetup(new TypeError('Failed to fetch'));
    expect($('[role=alert]').textContent).toBe('Warden could not look for account folders.');
  });

  it('shows the server’s refusal and stays open', async () => {
    await openSetup([WORK], 'That folder is already watched.');
    await submit();
    expect($('[role=alert]').textContent).toBe('That folder is already watched.');
    expect(document.querySelector('form')).not.toBeNull();
  });

  it('waits while the Claude app is signed out: says so in amber, explains Start, and lets Skip finish', async () => {
    const calls = await openSetup([WORK]);
    store.state!.problems = [{ kind: 'claudeAppSignedOut' }];
    store.state!.main!.identity = null;
    await flushPromises();
    const main = $('section[aria-labelledby=setup-main]');
    expect(main.querySelector('.text-attn')!.textContent).toBe('The Claude app is signed out. Sign in to it.');
    expect(main.textContent).toContain('Setup carries on by itself once the Claude app is signed in.');
    const start = button(/^Start$/);
    expect(start.getAttribute('aria-disabled')).toBe('true');
    expect(start.disabled).toBe(false);
    start.click();
    await submit();
    expect(calls.filter((c) => c.path === '/api/setup')).toEqual([]);
    start.focus();
    await flushPromises();
    expect(document.querySelector('[role=tooltip]')?.textContent).toContain('You can skip setup for now');
    // The app signs in: the screen updates by itself.
    store.state!.problems = [];
    store.state!.main!.identity = { email: 'main@example.com', organization: null };
    await flushPromises();
    expect(button(/^Start$/).getAttribute('aria-disabled')).toBeNull();
    button(/^Skip for now$/).click();
    await flushPromises();
    expect(calls.at(-1)!.body).toEqual({ accounts: [], skip: true });
  });

  it('is not shown once setup is done', async () => {
    vi.stubGlobal('fetch', async () => new Response('[]', { status: 200 }));
    store.state = state([]);
    mount(SetupModal, { attachTo: document.body });
    await flushPromises();
    expect(document.querySelector('form')).toBeNull();
  });

  it('keeps the title and buttons in place while the sections between them scroll', async () => {
    await openSetup([WORK, OUT]);
    const body = $('[data-setup-body]');
    expect(body.className).toContain('overflow-y-auto');
    expect(body.contains($('h2'))).toBe(false);
    expect(body.contains(button(/^Start$/))).toBe(false);
    expect(body.contains($('section[aria-labelledby=setup-others]'))).toBe(true);
    // With nothing below (no layout here), no divider shows above the buttons.
    expect($('[data-setup-buttons]').className).toContain('border-transparent');
  });
});
