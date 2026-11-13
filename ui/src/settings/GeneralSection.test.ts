import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { go, route } from '../route';
import { store } from '../store';
import type { State } from '../api';
import { fakeServer } from '../test/api';
import { state } from '../test/state';
import GeneralSection from './GeneralSection.vue';

enableAutoUnmount(afterEach);

afterEach(() => {
  store.state = null;
  vi.unstubAllGlobals();
  go('');
});

function setup(over: Partial<State> = {}, answer?: () => State | { status: number; error: string }) {
  store.state = { ...state([]), ...over };
  const calls = fakeServer(answer ?? (() => store.state!));
  return { page: mount(GeneralSection, { attachTo: document.body }), calls };
}

describe('GeneralSection', () => {
  it('saves the usage interval and the theme as numbers and names', async () => {
    const { page, calls } = setup();
    const interval = page.get('[role=radiogroup][aria-label="Read usage every"]');
    expect(interval.find('[aria-checked=true]').text()).toBe('5 min');
    await interval.findAll('[role=radio]')[3]!.trigger('click');
    await page.get('[role=radiogroup][aria-label=Theme]').findAll('[role=radio]')[2]!.trigger('click');
    await flushPromises();
    expect(calls.map((c) => c.body)).toEqual([{ usageIntervalMin: 30 }, { theme: 'dark' }]);
  });

  it('shows the tray and Start with Windows switches only in the desktop app, and saves them', async () => {
    expect(setup().page.find('#keep-in-tray').exists()).toBe(false);

    const { page, calls } = setup({ desktop: { startWithWindows: false } });
    expect(page.text()).toContain('Closing the window quits Warden.');
    await page.get('#keep-in-tray').trigger('click');
    await page.get('#start-with-windows').trigger('click');
    await flushPromises();
    expect(calls.map((c) => c.body)).toEqual([{ keepInTray: true }, { startWithWindows: true }]);
  });

  it('shows the log folder, and opens it only in the desktop app', async () => {
    const open = (p: ReturnType<typeof setup>['page']) =>
      p.findAll('button').find((b) => b.text() === 'Open log folder');
    const browser = setup().page;
    expect(browser.text()).toContain(store.state!.logDir);
    expect(open(browser)).toBeUndefined();

    const { page, calls } = setup({ desktop: { startWithWindows: false } });
    await open(page)!.trigger('click');
    await flushPromises();
    expect(calls).toEqual([{ method: 'POST', path: '/api/logs/open', body: undefined }]);
  });

  it('runs setup again and closes Settings, or stays and says why when the server refuses', async () => {
    go('settings/general');
    const ok = setup();
    await ok.page
      .findAll('button')
      .find((b) => b.text() === 'Run setup again')!
      .trigger('click');
    await flushPromises();
    expect(ok.calls).toEqual([{ method: 'POST', path: '/api/setup/restart', body: undefined }]);
    expect(route.settingsOpen.value).toBe(false);

    go('settings/general');
    await flushPromises();
    const refused = setup({}, () => ({ status: 409, error: 'Setup is already running.' }));
    await refused.page
      .findAll('button')
      .find((b) => b.text() === 'Run setup again')!
      .trigger('click');
    await flushPromises();
    expect(refused.page.get('[role=alert]').text()).toBe('Setup is already running.');
    expect(route.settingsOpen.value).toBe(true);
  });
});
