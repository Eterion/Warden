import { mount } from '@vue/test-utils';
import { afterEach, describe, expect, it } from 'vitest';
import { store } from '../store';
import { NOW, account, state } from '../test/state';
import TopStrip from './TopStrip.vue';

afterEach(() => {
  store.state = null;
  store.connected = true;
});

describe('TopStrip', () => {
  const two = () => state([account({ id: 'main', label: 'Main' }), account({ id: 's', label: 'Second' })]);
  const H = 3_600_000;

  it('says the work stays on main while its week resets soonest, with the order on hover', () => {
    store.now = NOW;
    store.state = two();
    store.state.delegation = {
      ...store.state.delegation,
      mainWorksItself: true,
      handOffAt: 98,
      order: [
        { accountId: 'main', deadline: NOW + 10 * H, reset: false },
        { accountId: 's', deadline: NOW + 50 * H, reset: false },
      ],
    };
    const bar = mount(TopStrip);
    const item = bar.get('[data-work]');
    expect(item.element.tagName).toBe('BUTTON');
    expect(item.text().replace(/\s+/g, ' ')).toContain('Work stays on Main: its week resets soonest');
    expect(bar.findAllComponents({ name: 'WTooltip' }).map((w) => w.props('text'))).toEqual([
      'Main (resets in 10 h; keeps 2% for you), then Second (3 days)',
    ]);
    expect(item.classes()).not.toContain('text-attn');
  });

  it('names the other account once main delegates, with the order for screen readers too', () => {
    store.now = NOW;
    store.state = two();
    store.state.delegation = {
      mainWorksItself: false,
      delegated: { accountId: 's' },
      why: 'soonest',
      handOffAt: 80,
      order: [
        { accountId: 's', deadline: NOW + 15 * H, reset: false },
        { accountId: 'main', deadline: NOW + 93 * H, reset: false },
      ],
      suggestedReset: null,
    };
    const item = mount(TopStrip).get('[data-work]');
    expect(item.text().replace(/\s+/g, ' ')).toContain('Work goes to Second: its week resets in 15 h');
    expect(item.get('.sr-only').text()).toBe('. Second (resets in 15 h), then Main (4 days; keeps 20% for you)');
  });

  it('says the work stays on main when no other account has room', () => {
    store.state = state([account({ id: 'main', label: 'Main' })]);
    store.state.delegation = {
      mainWorksItself: true,
      delegated: null,
      why: 'noOther',
      handOffAt: 90,
      order: [],
      suggestedReset: null,
    };
    const item = mount(TopStrip).get('[data-work]');
    expect(item.text().replace(/\s+/g, ' ')).toContain('Work stays on Main: no other account has room');
  });

  it('asks in amber for a limit reset to be used first, and says where the work goes meanwhile', () => {
    store.now = NOW;
    store.state = two();
    const at = NOW + 2 * H;
    store.state.delegation = {
      mainWorksItself: true,
      delegated: { accountId: 's' },
      why: 'mainSoonest',
      handOffAt: 80,
      order: [
        { accountId: 's', deadline: at, reset: true },
        { accountId: 'main', deadline: NOW + 93 * H, reset: false },
      ],
      suggestedReset: { accountId: 's', resetId: 'r', title: 'Full reset', expiresAt: at },
    };
    const bar = mount(TopStrip);
    const item = bar.get('[data-work]');
    expect(item.text().replace(/\s+/g, ' ')).toContain("Use Second's Full reset (expires today) to give it the work");
    expect(item.classes()).toContain('text-attn');
    expect(bar.findComponent({ name: 'WTooltip' }).props('text')).toBe(
      'Until then, work stays on Main. Second (its Full reset expires today), then Main (4 days; keeps 20% for you)',
    );
  });

  it('says plainly, in grey, when hooks are off for this run, and nothing otherwise', () => {
    store.state = state([account({ id: 'main', label: 'Main' })]);
    expect(mount(TopStrip).text()).not.toContain('Hooks are off');
    store.state.hooksOff = true;
    const notice = mount(TopStrip)
      .findAll('span')
      .find((s) => s.text() === 'Hooks are off for this run')!;
    expect(notice.classes()).toContain('text-secondary');
  });

  it('says how long ago usage was read', () => {
    store.now = NOW;
    store.state = state([account({ id: 'main', label: 'Main' })]);
    expect(mount(TopStrip).text()).toContain('Usage read 1 min ago');
  });
});
