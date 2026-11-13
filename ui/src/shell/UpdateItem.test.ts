import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { State, Update } from '../api';
import { store } from '../store';
import { fakeServer } from '../test/api';
import { state } from '../test/state';
import UpdateItem from './UpdateItem.vue';

enableAutoUnmount(afterEach);

afterEach(() => {
  store.state = null;
  vi.unstubAllGlobals();
});

function setup(update: Update | null, answer?: () => State | { status: number; error: string }) {
  store.state = { ...state([]), update };
  const calls = fakeServer(answer ?? (() => store.state!));
  return { item: mount(UpdateItem, { attachTo: document.body }), calls };
}

const READY: Update = { version: '0.2.0', downloaded: 100, ready: true };
const button = (name: string) =>
  [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === name);

describe('UpdateItem', () => {
  it('shows nothing while there is no update', () => {
    expect(setup(null).item.html()).toBe('<!--v-if-->');
  });

  it('shows the download with its version and progress, and no button yet', () => {
    const { item } = setup({ version: '0.2.0', downloaded: 45, ready: false });
    expect(item.text()).toContain('Downloading update');
    expect(item.text()).toContain('0.2.0');
    const bar = item.get('[role=progressbar]');
    expect(bar.attributes('aria-label')).toBe('Downloading Warden 0.2.0');
    expect(bar.attributes('aria-valuenow')).toBe('45');
    expect(bar.get('span').attributes('style')).toContain('width: 45%');
    expect(item.find('button').exists()).toBe(false);
    // Before the download starts, the bar is empty and says no value.
    const waiting = setup({ version: '0.2.0', downloaded: null, ready: false }).item;
    expect(waiting.get('[role=progressbar]').attributes('aria-valuenow')).toBeUndefined();
  });

  it('asks before installing, and cancelling installs nothing', async () => {
    const { item, calls } = setup(READY);
    const install = item.get('button');
    expect(install.text()).toBe('Install and restart');
    expect(install.attributes('aria-label')).toBe('Install Warden 0.2.0 and restart');
    await install.trigger('click');
    await flushPromises();
    expect(document.body.textContent).toContain('Install Warden 0.2.0?');
    expect(document.body.textContent).toContain('Warden closes, installs the update and starts again.');
    button('Cancel')!.click();
    await flushPromises();
    expect(document.body.textContent).not.toContain('Warden closes, installs the update');
    expect(calls).toEqual([]);
  });

  it('installs on yes, then blocks the app while the update installs', async () => {
    const { item, calls } = setup(READY);
    await item.get('button').trigger('click');
    await flushPromises();
    document.querySelector<HTMLFormElement>('form')!.requestSubmit();
    await flushPromises();
    expect(calls).toEqual([{ method: 'POST', path: '/api/update/install', body: undefined }]);
    const status = document.querySelector('[role=status]')!;
    expect(status.textContent).toContain('Installing Warden 0.2.0');
    expect(status.textContent).toContain('Warden starts again when it is done.');
    expect(document.activeElement).toBe(status);
    // Neither Escape nor a click beside it closes it.
    document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await flushPromises();
    expect(document.querySelector('[role=status]')).not.toBeNull();
    expect(button('Cancel')).toBeUndefined();
  });

  it('says why when the update cannot be installed, and asks again', async () => {
    const { item } = setup(READY, () => ({ status: 409, error: 'No update is ready to install.' }));
    await item.get('button').trigger('click');
    await flushPromises();
    document.querySelector<HTMLFormElement>('form')!.requestSubmit();
    await flushPromises();
    expect(document.querySelector('[role=alert]')?.textContent).toBe('No update is ready to install.');
    expect(document.querySelector('[role=status]')).toBeNull();
    expect(button('Cancel')).toBeDefined();
  });
});
