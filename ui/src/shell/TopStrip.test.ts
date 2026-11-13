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

  it('names the account that gets the work now: main, until its hand-off point, with its reason', () => {
    store.now = NOW;
    store.state = two();
    store.state.delegation = { ...store.state.delegation, mainWorksItself: true, handOffAt: 98 };
    const bar = mount(TopStrip);
    const item = bar.get('[data-work]');
    expect(item.element.tagName).toBe('BUTTON');
    expect(item.text().replace(/\s+/g, ' ')).toContain('Work goes to Main until 98%');
    expect(bar.findAllComponents({ name: 'WTooltip' }).map((w) => w.props('text'))).toEqual(['Main has room.']);
    expect(bar.text()).not.toContain('Delegated work');
  });

  it('names the other account once main hands off, with both reasons for screen readers too', () => {
    store.state = two();
    store.state.delegation = {
      mainWorksItself: false,
      delegated: { accountId: 's' },
      mainReason: 'Main is near its reserve.',
      delegatedReason: 'Second has the most allowance left.',
      handOffAt: 80,
    };
    const item = mount(TopStrip).get('[data-work]');
    expect(item.text()).toContain('Work goes to');
    expect(item.text()).toContain('Second');
    expect(item.text()).not.toContain('until');
    expect(item.get('.sr-only').text()).toBe('. Main is near its reserve. Second has the most allowance left.');
  });

  it('says work goes to main, with no "until", when no account can take work', () => {
    store.state = state([account({ id: 'main', label: 'Main' })]);
    store.state.delegation = {
      mainWorksItself: true,
      delegated: null,
      mainReason: 'No account can take delegated work, so main keeps it.',
      delegatedReason: 'No account can take delegated work, so main keeps it.',
      handOffAt: 90,
    };
    const item = mount(TopStrip).get('[data-work]');
    expect(item.text().replace(/\s+/g, ' ')).toContain('Work goes to Main');
    expect(item.text()).not.toContain('until');
  });

  it("names the account work goes to while main's usage is unknown", () => {
    store.state = two();
    store.state.delegation = {
      mainWorksItself: false,
      delegated: { accountId: 's' },
      mainReason: "Main's usage is unknown, so work goes to the other accounts until Warden is signed in to main.",
      delegatedReason: 'Second has the most allowance left.',
      handOffAt: 75,
    };
    const bar = mount(TopStrip);
    expect(bar.get('[data-work]').text()).toContain('Second');
    expect(bar.findComponent({ name: 'WTooltip' }).props('text')).toContain("Main's usage is unknown");
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
