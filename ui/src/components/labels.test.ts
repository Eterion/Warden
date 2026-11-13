import { mount } from '@vue/test-utils';
import { defineComponent } from 'vue';
import { describe, expect, it } from 'vitest';
import { vLabelState } from './labels';

const Field = defineComponent({
  directives: { labelState: vLabelState },
  template: `<div><label for="f">Name</label><input id="f" v-label-state /><label>Wrapped <input id="g" type="checkbox" v-label-state /></label><span id="l">Keep for my own work</span><span role="slider" tabindex="0" aria-labelledby="l" v-label-state></span></div>`,
});

describe('v-label-state', () => {
  it('marks a control while its label is hovered or pressed, for a label beside it or around it', async () => {
    const page = mount(Field, { attachTo: document.body });
    const [beside, around] = page.findAll('label');
    const input = page.get('#f');
    await beside!.trigger('pointerenter');
    expect(input.attributes('data-label-hover')).toBe('');
    await beside!.trigger('pointerdown', { button: 0 });
    expect(input.attributes('data-label-press')).toBe('');
    await beside!.trigger('pointerup');
    expect(input.attributes('data-label-press')).toBeUndefined();
    await beside!.trigger('pointerleave');
    expect(input.attributes('data-label-hover')).toBeUndefined();

    await around!.trigger('pointerenter');
    expect(page.get('#g').attributes('data-label-hover')).toBe('');
    page.unmount();
  });

  it('marks a control HTML cannot label from the element its aria-labelledby names, and focuses it on a click', async () => {
    const page = mount(Field, { attachTo: document.body });
    const label = page.get('#l');
    const thumb = page.get('[role=slider]');
    await label.trigger('pointerenter');
    expect(thumb.attributes('data-label-hover')).toBe('');
    await label.trigger('pointerdown', { button: 0 });
    expect(thumb.attributes('data-label-press')).toBe('');
    await label.trigger('click');
    expect(document.activeElement).toBe(thumb.element);
    await label.trigger('pointerleave');
    expect(thumb.attributes('data-label-hover')).toBeUndefined();
    page.unmount();
  });
});
