import { describe, expect, it } from 'vitest';
import { nextTick, ref } from 'vue';
import { keptWhileClosing } from './kept';

describe('keptWhileClosing', () => {
  it('follows the value while open and keeps the last one while closed', async () => {
    const open = ref(true);
    const section = ref('accounts');
    const kept = keptWhileClosing(
      () => open.value,
      () => section.value,
    );
    expect(kept.value).toBe('accounts');
    section.value = 'general';
    await nextTick();
    expect(kept.value).toBe('general');
    // Closing clears the state at once; the closing modal still draws what it showed.
    section.value = 'accounts';
    await nextTick();
    open.value = false;
    section.value = 'general';
    await nextTick();
    expect(kept.value).toBe('accounts');
    // Opened again: the new value at once.
    open.value = true;
    await nextTick();
    expect(kept.value).toBe('general');
  });
});
