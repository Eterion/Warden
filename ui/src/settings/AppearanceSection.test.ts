import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { store } from '../store';
import { fakeServer } from '../test/api';
import { state } from '../test/state';
import AppearanceSection from './AppearanceSection.vue';

enableAutoUnmount(afterEach);

afterEach(() => {
  store.state = null;
  vi.unstubAllGlobals();
});

function setup() {
  store.state = state([]);
  const calls = fakeServer(() => store.state!);
  return { page: mount(AppearanceSection), calls };
}

describe('AppearanceSection', () => {
  it('offers System, Light and Dark and saves the selected theme by name', async () => {
    const { page, calls } = setup();
    const theme = page.get('[role=radiogroup][aria-label=Theme]');
    const choices = theme.findAll('[role=radio]');
    expect(choices.map((choice) => choice.text())).toEqual(['System', 'Light', 'Dark']);
    expect(theme.get('[aria-checked=true]').text()).toBe('System');
    await choices[2]!.trigger('click');
    await flushPromises();
    expect(calls).toEqual([{ method: 'PUT', path: '/api/settings', body: { theme: 'dark' } }]);
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
});
