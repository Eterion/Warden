import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { defineComponent, h, ref } from 'vue';
import type { Candidate } from '../api';
import { desktop } from '../desktop';
import { store } from '../store';
import type { Call } from '../test/api';
import { account, state } from '../test/state';
import AddAccountModal from './AddAccountModal.vue';

enableAutoUnmount(afterEach);

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: vi.fn(async (cmd: string) => (cmd === 'pick_folder' ? 'PICKED' : null)),
}));

afterEach(() => {
  store.state = null;
  desktop.inApp = false;
  vi.unstubAllGlobals();
});

function candidate(c: Partial<Candidate> & Pick<Candidate, 'folder' | 'label'>): Candidate {
  return { provider: 'claude', identity: null, signedIn: true, known: false, ...c };
}

/** The server: the found folders on GET, otherwise the state, or `refuse` once set. */
function server(found: Candidate[], refuse: { current: string | null } = { current: null }): Call[] {
  const calls: Call[] = [];
  vi.stubGlobal('fetch', async (path: string, init: RequestInit = {}) => {
    const method = init.method ?? 'GET';
    calls.push({ method, path, body: typeof init.body === 'string' ? JSON.parse(init.body) : undefined });
    if (method === 'GET') return new Response(JSON.stringify(found), { status: 200 });
    if (refuse.current) return new Response(JSON.stringify({ error: refuse.current }), { status: 400 });
    return new Response(JSON.stringify(store.state), { status: 200 });
  });
  return calls;
}

async function openModal() {
  const open = ref(false);
  mount(
    defineComponent({
      setup: () => () => h(AddAccountModal, { open: open.value, 'onUpdate:open': (v: boolean) => (open.value = v) }),
    }),
    { attachTo: document.body },
  );
  open.value = true;
  await flushPromises();
  return open;
}

const $ = <T extends Element = HTMLElement>(sel: string) => document.querySelector<T>(sel)!;
const buttons = () => [...document.querySelectorAll<HTMLButtonElement>('button')];
const button = (text: string) => buttons().find((b) => b.textContent!.trim() === text);
const radio = (title: string) =>
  [...document.querySelectorAll<HTMLElement>('[role=radio]')].find((r) => r.textContent!.trim().startsWith(title))!;
async function submit() {
  $('form').dispatchEvent(new Event('submit', { cancelable: true }));
  await flushPromises();
}
async function type(sel: string, text: string) {
  const input = $<HTMLInputElement>(sel);
  input.value = text;
  input.dispatchEvent(new Event('input'));
  await flushPromises();
}

describe('AddAccountModal', () => {
  it('names a new account after the ones of its provider, previews its folder, and creates it', async () => {
    store.state = state([account({ id: 'main', label: 'Main' })]);
    const calls = server([]);
    const open = await openModal();
    expect($<HTMLInputElement>('#add-name').value).toBe('Claude 2');
    expect(document.body.textContent).toContain('%USERPROFILE%\\.warden\\claude-2');
    await type('#add-name', 'Work Laptop');
    expect(document.body.textContent).toContain('%USERPROFILE%\\.warden\\claude-work-laptop');
    expect(button('Create and sign in')).toBeDefined();
    await submit();
    expect(calls.at(-1)).toEqual({
      method: 'POST',
      path: '/api/accounts/new',
      body: { provider: 'claude', label: 'Work Laptop' },
    });
    expect(open.value).toBe(false);
  });

  it('switching to Codex renames the default and previews a codex folder under .warden', async () => {
    store.state = state([account({ id: 'main', label: 'Main' })]);
    const calls = server([]);
    await openModal();
    document.querySelectorAll<HTMLElement>('[role=radio]')[1]!.click();
    await flushPromises();
    expect($<HTMLInputElement>('#add-name').value).toBe('Codex 1');
    expect(document.body.textContent).toContain('%USERPROFILE%\\.warden\\codex-1');
    await submit();
    expect(calls.at(-1)!.body).toEqual({ provider: 'codex', label: 'Codex 1' });
  });

  it('offers only signed-in folders of the provider that are not added yet, and a pick fills folder and name', async () => {
    store.state = state([account({ id: 'main', label: 'Main' })]);
    const calls = server([
      candidate({ folder: '%USERPROFILE%\\.claude-work', label: 'Work' }),
      candidate({ folder: '%USERPROFILE%\\.claude', label: 'Main', known: true }),
      candidate({ folder: '%USERPROFILE%\\.claude-old', label: 'Old', signedIn: false }),
      candidate({ folder: '%USERPROFILE%\\.codex-work', label: 'Codex', provider: 'codex' }),
    ]);
    await openModal();
    radio('Existing folder').click();
    await flushPromises();
    const chips = [...document.querySelectorAll('[aria-labelledby=found-label] button')];
    expect(chips.map((c) => c.textContent!.trim())).toEqual(['%USERPROFILE%\\.claude-work']);
    (chips[0] as HTMLElement).click();
    await flushPromises();
    expect($<HTMLInputElement>('#add-folder').value).toBe('%USERPROFILE%\\.claude-work');
    expect($<HTMLInputElement>('#add-name').value).toBe('Work');
    expect(chips[0]!.getAttribute('aria-pressed')).toBe('true');
    await submit();
    expect(calls.at(-1)).toEqual({
      method: 'POST',
      path: '/api/accounts',
      body: { provider: 'claude', folder: '%USERPROFILE%\\.claude-work', label: 'Work' },
    });
  });

  it('asks for an existing folder when none was found, and stays open with the server’s refusal', async () => {
    store.state = state([account({ id: 'main', label: 'Main' })]);
    const refuse = { current: 'That folder has no signed-in Claude login.' as string | null };
    server([], refuse);
    const open = await openModal();
    radio('Existing folder').click();
    await flushPromises();
    expect($('#add-folder').getAttribute('placeholder')).toBe('Path of a Claude folder');
    await type('#add-folder', '%USERPROFILE%\\empty');
    await submit();
    expect($('[role=alert]').textContent).toBe('That folder has no signed-in Claude login.');
    expect(open.value).toBe(true);
    expect(button('Add folder')).toBeDefined();
  });

  it('starts afresh each time it opens', async () => {
    store.state = state([account({ id: 'main', label: 'Main' })]);
    server([]);
    const open = await openModal();
    radio('Existing folder').click();
    await flushPromises();
    await type('#add-folder', '%USERPROFILE%\\x');
    open.value = false;
    await flushPromises();
    open.value = true;
    await flushPromises();
    expect(document.querySelector('#add-folder')).toBeNull();
    expect($<HTMLInputElement>('#add-name').value).toBe('Claude 2');
  });

  it('a request still being sent when it closes never closes a later opening, which sends no second one meanwhile', async () => {
    store.state = state([account({ id: 'main', label: 'Main' })]);
    let answer: (r: Response) => void = () => {};
    let posts = 0;
    vi.stubGlobal('fetch', async (_path: string, init: RequestInit = {}) => {
      if ((init.method ?? 'GET') === 'GET') return new Response('[]', { status: 200 });
      posts++;
      return new Promise<Response>((r) => (answer = r));
    });
    const open = await openModal();
    await submit();
    expect(button('Create and sign in')!.getAttribute('aria-busy')).toBe('true');
    open.value = false;
    await flushPromises();
    open.value = true;
    await flushPromises();
    // Still being added: the new opening says so and sends nothing more.
    expect(button('Create and sign in')!.getAttribute('aria-busy')).toBe('true');
    await submit();
    expect(posts).toBe(1);

    answer(new Response(JSON.stringify(store.state), { status: 200 }));
    await flushPromises();
    expect(open.value).toBe(true);
    expect(document.querySelector('#add-name')).not.toBeNull();
    expect(button('Create and sign in')!.hasAttribute('aria-busy')).toBe(false);
  });

  it('chooses between a new account and an existing folder with a segmented control, not cards', async () => {
    await openModal();
    const how = document.querySelector('[role=radiogroup][aria-label="How to add"]')!;
    expect([...how.querySelectorAll('[role=radio]')].map((r) => r.textContent!.trim())).toEqual([
      'New account',
      'Existing folder',
    ]);
    // Claude or Codex stays as cards: two of them, each with its company.
    const cards = [...document.querySelectorAll('[role=radiogroup][aria-label=Provider] [role=radio]')];
    expect(cards).toHaveLength(2);
    for (const [card, [title, company]] of cards.map(
      (c, i) => [c, i ? ['Codex', 'OpenAI'] : ['Claude', 'Anthropic']] as const,
    )) {
      expect(card.textContent).toContain(title);
      expect(card.textContent).toContain(company);
    }
    expect(document.body.textContent).not.toContain('Warden creates it and opens');
  });
});
