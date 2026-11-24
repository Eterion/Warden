import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { watch } from 'vue';
import type { Theme } from '../api';
import { desktop } from '../desktop';
import { closeModal, go, openSettings, route } from '../route';
import AppearanceSection from '../settings/AppearanceSection.vue';
import SettingsModal from '../settings/SettingsModal.vue';
import { apply, store } from '../store';
import { fakeServer } from '../test/api';
import { account, state } from '../test/state';
import AppSidebar from './AppSidebar.vue';

vi.mock('../desktop', async (real) => ({ ...(await real<typeof import('../desktop')>()), quit: vi.fn() }));
vi.mock('../route', async (real) => {
  const r = await real<typeof import('../route')>();
  return { ...r, openSettings: vi.fn(r.openSettings) };
});

afterEach(() => {
  desktop.inApp = false;
  store.state = null;
  store.connected = true;
  closeModal();
  delete document.documentElement.dataset.theme;
  vi.mocked(openSettings).mockClear();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function systemTheme(initial: 'light' | 'dark') {
  const query = '(prefers-color-scheme: dark)';
  const matchMedia = window.matchMedia.bind(window);
  const media = matchMedia(query);
  let dark = initial === 'dark';
  Object.defineProperty(media, 'matches', { get: () => dark });
  vi.spyOn(window, 'matchMedia').mockImplementation((q) => (q === query ? media : matchMedia(q)));
  return (theme: 'light' | 'dark') => {
    dark = theme === 'dark';
    media.dispatchEvent(new MediaQueryListEvent('change', { media: query, matches: dark }));
  };
}

function themeServer(theme: Theme) {
  const initial = state([]);
  apply({ ...initial, settings: { ...initial.settings, theme } });
  const calls = fakeServer(() => ({
    ...initial,
    settings: { ...initial.settings, ...(calls.at(-1)?.body as { theme: Theme }) },
  }));
  return calls;
}

describe('AppSidebar', () => {
  async function openMenu(bar = mount(AppSidebar, { attachTo: document.body })) {
    desktop.inApp = true;
    await flushPromises();
    // From the keyboard: the test DOM has no pointer events for Reka's trigger.
    await bar.get('[aria-label="Warden menu"]').trigger('keydown', { key: 'Enter' });
    await flushPromises();
    return bar;
  }

  const item = (bar: Awaited<ReturnType<typeof openMenu>>, label: string) => {
    const found = bar.findAll('[role=menuitem]').find((i) => i.text() === label);
    if (!found) throw new Error(`no menu item ${label}`);
    return found;
  };

  /** Picks a menu item; Settings opens a couple of timer turns after the menu has closed. */
  const choose = async (bar: Awaited<ReturnType<typeof openMenu>>, label: string) => {
    await item(bar, label).trigger('click');
    await flushPromises();
    for (let i = 0; i < 3; i++) await new Promise((r) => setTimeout(r, 0));
    await flushPromises();
  };

  const escape = async () => {
    document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await flushPromises();
    // Reka gives focus back to a menu's button on the next tick.
    await new Promise((r) => setTimeout(r, 0));
  };

  it("groups App, Appearance and Accounts under Settings in Warden's menu, then Quit after a divider", async () => {
    const bar = await openMenu();
    const group = bar.get('[role=menu] [role=group]');
    expect(group.element.firstElementChild?.textContent).toBe('Settings');
    expect(group.element.firstElementChild?.id).toBe(group.attributes('aria-labelledby'));
    expect(group.findAll('[role=menuitem]').map((i) => i.text())).toEqual(['App', 'Appearance', 'Accounts']);
    const divider = bar.get('[role=separator]');
    expect(group.element.nextElementSibling).toBe(divider.element);
    expect(divider.element.nextElementSibling).toBe(item(bar, 'Quit Warden').element);
    bar.unmount();
  });

  it('opens each Settings section once the menu has closed, focus on its button by then', async () => {
    for (const [label, section] of [
      ['App', 'app'],
      ['Appearance', 'appearance'],
      ['Accounts', 'accounts'],
    ] as const) {
      const bar = await openMenu();
      const button = bar.get('[aria-label="Warden menu"]').element;
      // As Settings opens, the menu has closed and its button has focus: nothing moves focus after Settings takes it.
      let atOpen: { expanded: string | null; focus: Element | null } | null = null;
      const stop = watch(
        route.settingsOpen,
        (now) => {
          if (now) atOpen = { expanded: button.getAttribute('aria-expanded'), focus: document.activeElement };
        },
        { flush: 'sync' },
      );
      await choose(bar, label);
      stop();
      expect(route.section.value).toBe(section);
      expect(atOpen).toEqual({ expanded: 'false', focus: button });
      bar.unmount();
      closeModal();
    }
  });

  it("tells Settings to give focus back to the menu's button, since the menu item will be gone", async () => {
    const bar = await openMenu();
    const button = bar.get('[aria-label="Warden menu"]').element;
    await choose(bar, 'Accounts');
    const [section, returnTo] = vi.mocked(openSettings).mock.lastCall!;
    expect(section).toBe('accounts');
    expect(returnTo?.()).toBe(button);
    bar.unmount();
  });

  it('drops a menu choice that a newer action overtakes before it opens: Settings or another dialog opened, or the menu again', async () => {
    const turns = async () => {
      for (let i = 0; i < 3; i++) await new Promise((r) => setTimeout(r, 0));
      await flushPromises();
    };
    // Accounts chosen; before it opens, the sidebar's Settings opens App. App stays.
    let bar = await openMenu();
    await item(bar, 'Accounts').trigger('click');
    await flushPromises();
    openSettings();
    await turns();
    expect(route.section.value).toBe('app');
    bar.unmount();
    closeModal();

    // Accounts chosen; before it opens, the menu opens again. Nothing opens.
    bar = await openMenu();
    await item(bar, 'Accounts').trigger('click');
    await flushPromises();
    await openMenu(bar);
    await turns();
    expect(route.settingsOpen.value).toBe(false);
    bar.unmount();

    // Accounts chosen; before it opens, another dialog opens (a reset's confirmation). Settings does not open over it.
    bar = await openMenu();
    await item(bar, 'Accounts').trigger('click');
    await flushPromises();
    const other = document.createElement('div');
    other.setAttribute('role', 'dialog');
    document.body.append(other);
    await turns();
    expect(route.settingsOpen.value).toBe(false);
    other.remove();
    bar.unmount();
  });

  it('opens each section with focus inside; Escape gives it back to the button, and the menu still does after', async () => {
    store.state = state([account({ id: 'main', label: 'Main' })]);
    fakeServer(() => store.state!);
    const settings = mount(SettingsModal, { attachTo: document.body });
    desktop.inApp = true;
    const bar = mount(AppSidebar, { attachTo: document.body });
    const button = bar.get('[aria-label="Warden menu"]').element;
    for (const label of ['App', 'Appearance', 'Accounts']) {
      await openMenu(bar);
      await choose(bar, label);
      const dialog = document.querySelector('[role=dialog]');
      expect([...(dialog?.querySelectorAll('h2') ?? [])].map((h) => h.textContent)).toContain(label);
      expect(dialog?.contains(document.activeElement)).toBe(true);

      await escape();
      expect(document.querySelector('[role=dialog]')).toBeNull();
      expect(document.activeElement).toBe(button);
    }

    // The menu again, closed with Escape: focus is on its button again, not lost.
    await openMenu(bar);
    expect(bar.findAll('[role=menuitem]')).toHaveLength(4);
    await escape();
    expect(bar.findAll('[role=menuitem]')).toHaveLength(0);
    expect(document.activeElement).toBe(button);
    bar.unmount();
    settings.unmount();
  });

  it('opens Add account on its own from the button beside Accounts; closing it any way goes back to the page', async () => {
    store.state = state([account({ id: 'main', label: 'Main' })]);
    const calls = fakeServer(() => store.state!);
    const settings = mount(SettingsModal, { attachTo: document.body });
    const bar = mount(AppSidebar, { attachTo: document.body });
    const add = bar.get<HTMLButtonElement>('button[aria-label="Add account"]');

    for (const how of ['Cancel', 'Escape', 'a click beside it', 'an account added'] as const) {
      add.element.focus();
      await add.trigger('click');
      await flushPromises();
      expect(route.addAlone.value).toBe(true);
      expect(route.settingsOpen.value).toBe(false);
      const dialogs = [...document.querySelectorAll('[role=dialog]')];
      expect(dialogs.map((d) => d.querySelector('h2')?.textContent)).toEqual(['Add account']);
      const dialog = dialogs[0]!;
      expect(dialog.contains(document.activeElement)).toBe(true);

      if (how === 'Cancel') {
        [...dialog.querySelectorAll('button')].find((b) => b.textContent!.trim() === 'Cancel')!.click();
      } else if (how === 'Escape') {
        document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
      } else if (how === 'a click beside it') {
        // Reka listens for a press outside once the modal has opened (a timer turn later).
        await new Promise((r) => setTimeout(r, 0));
        const beside = bar.get('h2').element;
        beside.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
        beside.dispatchEvent(new PointerEvent('pointerup', { bubbles: true }));
        beside.dispatchEvent(new MouseEvent('click', { bubbles: true }));
      } else {
        dialog.querySelector('form')!.dispatchEvent(new Event('submit', { cancelable: true }));
      }
      await flushPromises();
      await new Promise((r) => setTimeout(r, 0));
      expect(route.addOpen.value, how).toBe(false);
      expect(route.settingsOpen.value, how).toBe(false);
      expect(location.hash, how).toBe('');
      expect(document.querySelector('[role=dialog]'), how).toBeNull();
      expect(document.activeElement, how).toBe(add.element);
    }
    expect(calls.filter((c) => c.method === 'POST')).toHaveLength(1);
    bar.unmount();
    settings.unmount();
  });

  it('still opens Add account over Settings › Accounts from there, and closing it goes back to Accounts', async () => {
    store.state = state([account({ id: 'main', label: 'Main' })]);
    fakeServer(() => store.state!);
    const settings = mount(SettingsModal, { attachTo: document.body });
    go('settings/accounts/add');
    await flushPromises();
    expect(route.addAlone.value).toBe(false);
    expect(route.settingsOpen.value).toBe(true);
    expect(document.querySelectorAll('[role=dialog]')).toHaveLength(2);

    const add = [...document.querySelectorAll('[role=dialog]')].find(
      (d) => d.querySelector('h2')?.textContent === 'Add account',
    )!;
    [...add.querySelectorAll('button')].find((b) => b.textContent!.trim() === 'Cancel')!.click();
    await flushPromises();
    expect(route.addOpen.value).toBe(false);
    expect(route.settingsOpen.value).toBe(true);
    expect(route.section.value).toBe('accounts');
    settings.unmount();
  });

  it('has no menu in the browser, only the name with the owl', () => {
    const bar = mount(AppSidebar);
    expect(bar.find('[aria-label="Warden menu"]').exists()).toBe(false);
    expect(bar.text()).toContain('Warden');
    expect(bar.find('img').attributes('alt')).toBe('');
    bar.unmount();
  });

  it.each([
    { system: 'light', setting: 'system', active: 'light', next: 'dark' },
    { system: 'dark', setting: 'system', active: 'dark', next: 'light' },
    { system: 'light', setting: 'light', active: 'light', next: 'dark' },
    { system: 'dark', setting: 'dark', active: 'dark', next: 'light' },
    { system: 'light', setting: 'dark', active: 'dark', next: 'system' },
    { system: 'dark', setting: 'light', active: 'light', next: 'system' },
  ] as const)(
    'switches $setting on a $system system to $next and saves it',
    async ({ system, setting, active, next }) => {
      systemTheme(system);
      const calls = themeServer(setting);
      const bar = mount(AppSidebar);
      const target = active === 'light' ? 'dark' : 'light';
      const button = bar.get(`button[aria-label="Switch to ${target} theme"]`);
      expect(button.get('svg').classes()).toContain(active === 'light' ? 'lucide-sun' : 'lucide-moon');

      await button.trigger('click');
      await flushPromises();
      expect(calls).toEqual([{ method: 'PUT', path: '/api/settings', body: { theme: next } }]);
      expect(store.state?.settings.theme).toBe(next);
      expect(document.documentElement.dataset.theme).toBe(next === 'system' ? undefined : next);
      expect(button.attributes('aria-label')).toBe(`Switch to ${active} theme`);
      expect(button.get('svg').classes()).toContain(target === 'light' ? 'lucide-sun' : 'lucide-moon');
      bar.unmount();
    },
  );

  it('follows system changes while open and stays in sync with Appearance in the desktop app', async () => {
    const changeSystem = systemTheme('light');
    desktop.inApp = true;
    const calls = themeServer('system');
    const bar = mount(AppSidebar);
    const appearance = mount(AppearanceSection);
    const button = bar.get('button[aria-label="Switch to dark theme"]');
    const radios = appearance.get('[role=radiogroup][aria-label=Theme]');

    changeSystem('dark');
    await flushPromises();
    expect(button.attributes('aria-label')).toBe('Switch to light theme');
    expect(button.get('svg').classes()).toContain('lucide-moon');
    expect(calls).toEqual([]);

    await button.trigger('click');
    await flushPromises();
    expect(radios.get('[aria-checked=true]').text()).toBe('Light');
    changeSystem('light');
    await flushPromises();
    expect(button.attributes('aria-label')).toBe('Switch to dark theme');

    await radios.findAll('[role=radio]')[2]!.trigger('click');
    await flushPromises();
    expect(button.attributes('aria-label')).toBe('Switch to light theme');
    expect(button.get('svg').classes()).toContain('lucide-moon');
    expect(calls.map((call) => call.body)).toEqual([{ theme: 'light' }, { theme: 'dark' }]);
    appearance.unmount();
    bar.unmount();
  });

  it.each([
    ['light', 'Switch to dark theme'],
    ['dark', 'Switch to light theme'],
  ] as const)('shows the tooltip on keyboard focus on a %s system', async (system, label) => {
    systemTheme(system);
    const bar = mount(AppSidebar, { attachTo: document.body });
    const button = bar.get<HTMLButtonElement>(`button[aria-label="${label}"]`);
    button.element.focus();
    await flushPromises();
    const tooltip = bar.get('[role=tooltip]');
    expect(tooltip.text()).toBe(label);
    expect(button.attributes('aria-describedby')).toBe(tooltip.attributes('id'));
    bar.unmount();
  });

  it('clears the theme error when Appearance saves a different theme', async () => {
    systemTheme('light');
    const initial = state([]);
    apply(initial);
    let refused = true;
    fakeServer(() =>
      refused
        ? { status: 409, error: 'Could not save the theme.' }
        : { ...initial, settings: { ...initial.settings, theme: 'dark' } },
    );
    const bar = mount(AppSidebar);
    await bar.get('[aria-label="Switch to dark theme"]').trigger('click');
    await flushPromises();
    expect(bar.get('[role=alert]').text()).toBe('Could not save the theme.');

    refused = false;
    const appearance = mount(AppearanceSection);
    await appearance.get('[aria-label=Theme]').findAll('[role=radio]')[2]!.trigger('click');
    await flushPromises();
    expect(store.state?.settings.theme).toBe('dark');
    expect(bar.find('[role=alert]').exists()).toBe(false);
    appearance.unmount();
    bar.unmount();
  });

  it('clears the theme error when Warden reconnects with the same theme', async () => {
    systemTheme('light');
    const initial = state([]);
    apply(initial);
    vi.stubGlobal('fetch', () => Promise.reject(new TypeError('Failed to fetch')));
    const bar = mount(AppSidebar);
    await bar.get('[aria-label="Switch to dark theme"]').trigger('click');
    await flushPromises();
    expect(store.connected).toBe(false);
    expect(bar.get('[role=alert]').text()).toBe('Warden could not be reached.');

    apply(initial);
    await flushPromises();
    expect(store.connected).toBe(true);
    expect(bar.find('[role=alert]').exists()).toBe(false);
    bar.unmount();
  });

  it('links Overview and Memory, marking the page shown, which stays marked under Settings', async () => {
    const bar = mount(AppSidebar);
    const current = () => bar.findAll('nav a[aria-current=page]').map((a) => a.text());
    expect(bar.findAll('nav a').map((a) => [a.text(), a.attributes('href')])).toEqual([
      ['Overview', '#'],
      ['Memory', '#/memory'],
    ]);
    go('memory');
    await flushPromises();
    expect(current()).toEqual(['Memory']);
    await bar
      .findAll('button')
      .find((button) => button.text() === 'Settings')!
      .trigger('click');
    await flushPromises();
    expect(location.hash).toBe('#/settings/app');
    expect(current()).toEqual(['Memory']);
    closeModal();
    expect(location.hash).toBe('#/memory');
    go('');
    await flushPromises();
    expect(current()).toEqual(['Overview']);
    bar.unmount();
  });
});
