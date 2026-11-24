import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { go, route } from '../route';
import { store } from '../store';
import type { State } from '../api';
import { fakeServer } from '../test/api';
import { state } from '../test/state';
import WSlider from '../components/WSlider.vue';
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

  it('shows Page transitions in the browser and saves it', async () => {
    const { page, calls } = setup();
    const toggle = page.get('button#page-transitions');
    expect(page.get('label[for=page-transitions]').text()).toBe('Page transitions');
    expect(toggle.attributes('aria-checked')).toBe('true');
    await toggle.trigger('click');
    await flushPromises();
    expect(calls).toEqual([{ method: 'PUT', path: '/api/settings', body: { pageTransitions: false } }]);
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

  it('saves how soon before a reset expires it warns, once per drag, and puts it back when refused', async () => {
    const { page, calls } = setup();
    const slider = page.getComponent(WSlider);
    expect(slider.props()).toMatchObject({ min: 1, max: 14, step: 1, labelledby: 'reset-warning-label' });
    expect(page.get('#reset-warning-label').text()).toBe('Warn before a reset expires');
    expect(page.get('[role=slider]').attributes('aria-labelledby')).toBe('reset-warning-label');
    expect(page.get('[data-reset-warning]').text()).toBe('3 days');
    slider.vm.$emit('update:modelValue', 2);
    slider.vm.$emit('update:modelValue', 1);
    await flushPromises();
    expect(calls).toEqual([]);
    expect(page.get('[data-reset-warning]').text()).toBe('1 day');
    expect(slider.props('valueText')).toBe('1 day');
    slider.vm.$emit('commit', 1);
    await flushPromises();
    expect(calls).toEqual([{ method: 'PUT', path: '/api/settings', body: { resetWarningDays: 1 } }]);

    const refused = setup({}, () => ({ status: 400, error: 'Warn from 1 to 14 days before a reset expires.' }));
    const other = refused.page.getComponent(WSlider);
    other.vm.$emit('update:modelValue', 9);
    other.vm.$emit('commit', 9);
    await flushPromises();
    expect(refused.page.get('[role=alert]').text()).toBe('Warn from 1 to 14 days before a reset expires.');
    expect(refused.page.get('[data-reset-warning]').text()).toBe('3 days');
  });
});
