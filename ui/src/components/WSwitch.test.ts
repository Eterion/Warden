import { mount } from '@vue/test-utils';
import { defineComponent } from 'vue';
import { describe, expect, it } from 'vitest';
import WSwitch from './WSwitch.vue';

describe('WSwitch', () => {
  it('reaches the shared switch’s button, with or without a reason it cannot be changed', async () => {
    for (const disabledReason of [undefined, 'Off for now']) {
      const page = mount(
        defineComponent({
          components: { WSwitch },
          data: () => ({ on: false, disabledReason }),
          template: `<div><label for="s">Start with Windows</label><WSwitch id="s" v-model="on" label="Start with Windows" :disabled-reason="disabledReason" /></div>`,
        }),
        { attachTo: document.body },
      );
      await page.get('label').trigger('pointerenter');
      const button = page.get('button#s');
      expect(button.attributes('role')).toBe('switch');
      expect(button.attributes('data-label-hover')).toBe('');
      page.unmount();
    }
  });
});
