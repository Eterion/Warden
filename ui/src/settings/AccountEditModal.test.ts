import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { defineComponent, h, ref } from 'vue';
import { store } from '../store';
import { fakeServer } from '../test/api';
import { account, state } from '../test/state';
import AccountEditModal from './AccountEditModal.vue';

enableAutoUnmount(afterEach);

afterEach(() => {
  store.state = null;
  vi.unstubAllGlobals();
});

const ALT = account({ id: 'a 1', label: 'Alt' });

/** Opens the modal the way Settings does: closed first, then open. */
async function openModal(
  mode: 'rename' | 'remove',
  answer?: () => ReturnType<typeof state> | { status: number; error: string },
) {
  store.state = state([ALT]);
  const calls = fakeServer(answer ?? (() => store.state!));
  const open = ref(false);
  mount(
    defineComponent({
      setup: () => () =>
        h(AccountEditModal, {
          open: open.value,
          'onUpdate:open': (v: boolean) => (open.value = v),
          account: ALT,
          mode,
        }),
    }),
    { attachTo: document.body },
  );
  open.value = true;
  await flushPromises();
  const form = document.querySelector('form')!;
  const submit = async () => {
    form.dispatchEvent(new Event('submit', { cancelable: true }));
    await flushPromises();
  };
  return { open, calls, form, submit };
}

describe('AccountEditModal', () => {
  it('renames: starts from the current name and saves the new one', async () => {
    const { open, calls, submit } = await openModal('rename');
    const input = document.getElementById('rename-name') as HTMLInputElement;
    expect(input.value).toBe('Alt');
    expect(input.maxLength).toBe(60);
    input.value = 'Work';
    input.dispatchEvent(new Event('input'));
    await submit();
    expect(calls).toEqual([{ method: 'PATCH', path: '/api/accounts/a%201', body: { label: 'Work' } }]);
    expect(open.value).toBe(false);
  });

  it('removes after saying what is kept, with Remove first', async () => {
    const { open, calls, form, submit } = await openModal('remove');
    expect(form.textContent).toContain('Remove Alt?');
    expect(form.textContent).toContain('The folder and its login stay as they are');
    expect([...form.querySelectorAll('button')].map((b) => b.textContent?.trim())).toEqual(['Remove', 'Cancel']);
    await submit();
    expect(calls).toEqual([{ method: 'DELETE', path: '/api/accounts/a%201', body: undefined }]);
    expect(open.value).toBe(false);
  });

  it("stays open with the server's sentence when it refuses", async () => {
    const { open, submit } = await openModal('rename', () => ({ status: 400, error: 'A name is needed.' }));
    await submit();
    expect(document.querySelector('[role=alert]')?.textContent).toBe('A name is needed.');
    expect(open.value).toBe(true);
  });

  it('Cancel closes without asking the server', async () => {
    const { open, calls, form } = await openModal('remove');
    [...form.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Cancel')!.click();
    await flushPromises();
    expect(open.value).toBe(false);
    expect(calls).toHaveLength(0);
  });
});
