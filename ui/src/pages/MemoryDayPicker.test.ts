import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { afterEach, describe, expect, it } from 'vitest';
import MemoryDayPicker from './MemoryDayPicker.vue';

const TODAY = '2026-11-09';
const RECORDED = ['2026-10-28', '2026-11-03', '2026-11-05', TODAY];

let picker: VueWrapper | null = null;

async function open(day = TODAY): Promise<VueWrapper> {
  picker = mount(MemoryDayPicker, { props: { day, today: TODAY, recorded: RECORDED }, attachTo: document.body });
  await picker.get('[data-day-picker]').trigger('click');
  await flushPromises();
  return picker;
}

const calendar = () => document.querySelector<HTMLElement>('[data-calendar]');
const cell = (iso: string) =>
  document.querySelector<HTMLElement>(`[data-calendar] [data-value="${iso}"]:not([data-outside-view])`)!;
const heading = () => document.querySelector('[data-month]')?.textContent?.trim();
const key = async (el: Element, k: string) => {
  el.dispatchEvent(new KeyboardEvent('keydown', { key: k, code: k, bubbles: true }));
  await flushPromises();
};

afterEach(() => {
  picker?.unmount();
  picker = null;
});

describe('MemoryDayPicker', () => {
  it('names the day on its button, the year only when not this one', () => {
    picker = mount(MemoryDayPicker, { props: { day: TODAY, today: TODAY, recorded: RECORDED } });
    expect(picker.get('[data-day]').text()).toBe('Monday, 9 November');
    picker.unmount();
    picker = mount(MemoryDayPicker, { props: { day: '2025-12-31', today: TODAY, recorded: RECORDED } });
    expect(picker.get('[data-day]').text()).toBe('Wednesday, 31 December 2025');
  });

  it('marks the recorded days, fills the day shown, rings today and disables the days to come', async () => {
    await open('2026-11-05');
    expect(calendar()).not.toBeNull();
    expect(heading()).toBe('November 2026');
    const marked = [...document.querySelectorAll('[data-calendar] :not([data-outside-view]) > [data-recorded]')].map(
      (dot) => dot.closest('[data-value]')!.getAttribute('data-value'),
    );
    expect(marked).toEqual(['2026-11-03', '2026-11-05', TODAY]);
    expect(cell('2026-11-03').getAttribute('aria-label')).toBe('Tuesday, 3 November, recorded');
    expect(cell('2026-11-04').getAttribute('aria-label')).toBe('Wednesday, 4 November');
    expect(cell('2026-11-05').hasAttribute('data-selected')).toBe(true);
    expect(cell(TODAY).getAttribute('aria-current')).toBe('date');
    expect(cell(TODAY).classList).toContain('inset-ring-indicator');
    expect(document.querySelectorAll('[data-calendar] [aria-current]')).toHaveLength(1);
    expect(cell(TODAY).hasAttribute('data-disabled')).toBe(false);
    for (const later of ['2026-11-10', '2026-11-30']) {
      expect(cell(later).hasAttribute('data-disabled')).toBe(true);
      expect(cell(later).getAttribute('aria-disabled')).toBe('true');
    }
  });

  it('goes back a month freely but no further forward than this one', async () => {
    await open();
    const next = () => document.querySelector<HTMLElement>('[data-calendar] button[aria-label="Next month"]')!;
    const prev = document.querySelector<HTMLElement>('[data-calendar] button[aria-label="Previous month"]')!;
    expect(next().getAttribute('aria-disabled')).toBe('true');
    next().click();
    await flushPromises();
    expect(heading()).toBe('November 2026');
    prev.click();
    await flushPromises();
    expect(heading()).toBe('October 2026');
    expect(cell('2026-10-28').querySelector('[data-recorded]')).not.toBeNull();
    expect(next().getAttribute('aria-disabled')).toBeNull();
    next().click();
    await flushPromises();
    expect(heading()).toBe('November 2026');
  });

  it('picks a day, closes and gives focus back to its button', async () => {
    const p = await open();
    cell('2026-11-03').click();
    await flushPromises();
    expect(p.emitted('pick')).toEqual([['2026-11-03']]);
    expect(calendar()).toBeNull();
    expect(document.activeElement).toBe(p.get('[data-day-picker]').element);
  });

  it('picking a day to come does nothing', async () => {
    const p = await open();
    cell('2026-11-12').click();
    await flushPromises();
    expect(p.emitted('pick')).toBeUndefined();
  });

  it('works from the keyboard: arrows between days, Page Up and Down between months, Enter to pick', async () => {
    const p = await open('2026-11-05');
    // Focus starts on the day shown.
    expect(document.activeElement).toBe(cell('2026-11-05'));
    await key(document.activeElement!, 'ArrowLeft');
    expect(document.activeElement).toBe(cell('2026-11-04'));
    // A week on is still to come: focus stays.
    await key(document.activeElement!, 'ArrowDown');
    expect(document.activeElement).toBe(cell('2026-11-04'));
    await key(document.activeElement!, 'PageUp');
    expect(heading()).toBe('October 2026');
    expect(document.activeElement?.getAttribute('data-value')).toBe('2026-10-04');
    await key(document.activeElement!, 'PageDown');
    expect(heading()).toBe('November 2026');
    expect(document.activeElement).toBe(cell('2026-11-04'));
    // Not past this month: the latest day it can go to is today.
    await key(document.activeElement!, 'PageDown');
    expect(heading()).toBe('November 2026');
    expect(document.activeElement).toBe(cell(TODAY));
    await key(document.activeElement!, 'ArrowLeft');
    await key(document.activeElement!, 'Enter');
    expect(p.emitted('pick')).toEqual([['2026-11-08']]);
    expect(calendar()).toBeNull();
    expect(document.activeElement).toBe(p.get('[data-day-picker]').element);
  });

  it('closes on Escape, focus back on its button', async () => {
    const p = await open();
    await key(document.activeElement!, 'Escape');
    expect(calendar()).toBeNull();
    expect(p.emitted('pick')).toBeUndefined();
    expect(document.activeElement).toBe(p.get('[data-day-picker]').element);
  });
});
