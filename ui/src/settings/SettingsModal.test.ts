import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils';
import { afterEach, describe, expect, it } from 'vitest';
import { go } from '../route';
import { store } from '../store';
import { account, state } from '../test/state';
import SettingsModal from './SettingsModal.vue';

enableAutoUnmount(afterEach);

afterEach(() => {
  store.state = null;
  go('');
});

async function open(path: string, desktop = false) {
  store.state = state([account({ id: 'main', label: 'Main' })]);
  if (desktop) store.state.desktop = { startWithWindows: false };
  go(path);
  mount(SettingsModal, { attachTo: document.body });
  await flushPromises();
  return document.querySelector('[role=dialog]')!;
}

describe('SettingsModal', () => {
  it.each(['settings', 'settings/general'])('opens App from %s', async (path) => {
    const dialog = await open(path);
    expect(dialog.querySelector('h2:not(.sr-only)')?.textContent).toBe('App');
    expect(dialog.querySelector('[aria-current=page]')?.textContent?.trim()).toBe('App');
    expect(dialog.querySelector('[aria-label="Read usage every"]')).not.toBeNull();
  });

  it('lists App, Appearance and Accounts in order and shows their own rows', async () => {
    const dialog = await open('settings/app', true);
    const items = [...dialog.querySelectorAll<HTMLButtonElement>('nav button')];
    const title = () => dialog.querySelector('h2:not(.sr-only)')?.textContent;
    const rows = () =>
      [...dialog.querySelector('section')!.children].map(
        (row) => row.firstElementChild?.firstElementChild?.textContent,
      );
    expect(items.map((item) => item.textContent?.trim())).toEqual(['App', 'Appearance', 'Accounts']);
    expect(rows()).toEqual([
      'Read usage every',
      'Warn before a reset expires',
      'Keep running in the tray',
      'Start with Windows',
      "Warden's data",
      'Log files',
    ]);

    items[1]!.click();
    await flushPromises();
    expect(location.hash).toBe('#/settings/appearance');
    expect(title()).toBe('Appearance');
    expect(rows()).toEqual(['Theme', 'Page transitions']);

    items[2]!.click();
    await flushPromises();
    expect(location.hash).toBe('#/settings/accounts');
    expect(title()).toBe('Accounts');
    expect(dialog.querySelector('[aria-label=Accounts]')).not.toBeNull();
    expect(dialog.querySelector('#reserve-label')?.textContent).toBe('Keep for my own work');
  });

  it.each([false, true])('finds the moved rows under their new sections (desktop=%s)', async (desktop) => {
    const dialog = await open('settings/app', desktop);
    const search = dialog.querySelector<HTMLInputElement>('input[type=search]')!;
    const findResult = async (label: string) => {
      search.value = label;
      search.dispatchEvent(new Event('input'));
      await flushPromises();
      return [...dialog.querySelectorAll<HTMLButtonElement>('li button')].find(
        (button) => button.firstElementChild?.firstElementChild?.textContent === label,
      );
    };
    const app = ['Read usage every', 'Warn before a reset expires', "Warden's data", 'Log files'];
    if (desktop) app.push('Keep running in the tray', 'Start with Windows');
    for (const label of app) {
      expect((await findResult(label))?.children[1]?.textContent?.trim()).toBe('App');
    }
    for (const label of ['Theme', 'Page transitions']) {
      expect((await findResult(label))?.children[1]?.textContent?.trim()).toBe('Appearance');
    }
    expect((await findResult('Add account'))?.children[1]?.textContent?.trim()).toBe('Accounts');
    (await findResult('Theme'))!.click();
    await flushPromises();
    expect(location.hash).toBe('#/settings/appearance');
    expect(search.value).toBe('');
  });
});
