import { flushPromises, mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import WSlider from './WSlider.vue';

describe('WSlider', () => {
  it('moves and commits with the keyboard', async () => {
    const page = mount(WSlider, { props: { modelValue: 5, label: 'Hours', min: 1, max: 10 }, attachTo: document.body });
    await page.get('[role=slider]').trigger('keydown', { key: 'ArrowRight' });
    expect(page.emitted('update:modelValue')).toEqual([[6]]);
    expect(page.emitted('commit')).toEqual([[6]]);
    page.unmount();
  });

  it('with a reason stays focusable, says why on focus, and ignores keys', async () => {
    const page = mount(WSlider, {
      props: { modelValue: 5, label: 'Hours', min: 1, max: 10, disabledReason: 'Off for now' },
      attachTo: document.body,
    });
    const thumb = page.get('[role=slider]');
    expect(thumb.attributes('tabindex')).toBe('0');
    expect(thumb.attributes('aria-disabled')).toBe('true');
    await thumb.trigger('focus');
    await flushPromises();
    expect(document.body.textContent).toContain('Off for now');
    await thumb.trigger('keydown', { key: 'ArrowRight' });
    expect(page.emitted('update:modelValue')).toBeUndefined();
    expect(page.emitted('commit')).toBeUndefined();
    page.unmount();
  });

  it('with range has two named thumbs, moves each, and commits the pair', async () => {
    const page = mount(WSlider, {
      props: {
        modelValue: [540, 1080],
        label: 'Work hours',
        range: true,
        thumbLabels: ['Start', 'End'],
        valueTexts: ['09:00', '18:00'],
        min: 0,
        max: 1440,
        step: 30,
      },
      attachTo: document.body,
    });
    const thumbs = page.findAll('[role=slider]');
    expect(thumbs.map((t) => t.attributes('aria-label'))).toEqual(['Start', 'End']);
    expect(thumbs.map((t) => t.attributes('aria-valuetext'))).toEqual(['09:00', '18:00']);
    await thumbs[0]!.trigger('keydown', { key: 'ArrowRight' });
    expect(page.emitted('update:modelValue')).toEqual([[[570, 1080]]]);
    expect(page.emitted('commit')).toEqual([[[570, 1080]]]);
    page.unmount();
  });
});
