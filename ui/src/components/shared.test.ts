// Behaviour of the shared components: keyboard, aria-*, focusable disabled controls and what a click does.
import { flushPromises, mount } from '@vue/test-utils';
import { Folder } from 'lucide-vue-next';
import { afterEach, describe, expect, it } from 'vitest';
import { defineComponent, h, nextTick, ref, type VNodeChild } from 'vue';
import AppModal from './AppModal.vue';
import NavItem from './NavItem.vue';
import RemovableChip from './RemovableChip.vue';
import SegmentedControl from './SegmentedControl.vue';
import ToggleChip from './ToggleChip.vue';
import UsageBar from './UsageBar.vue';
import WButton from './WButton.vue';
import WIconButton from './WIconButton.vue';
import WListItem from './WListItem.vue';
import WSwitch from './WSwitch.vue';
import WTextField from './WTextField.vue';

/** A component that renders `render`: the props bound to the test's own refs. */
const host = (render: () => VNodeChild) => defineComponent({ setup: () => render });

afterEach(() => {
  document.body.innerHTML = '';
});

describe('WSwitch', () => {
  it('turns on and off when clicked, saying so through aria-checked', async () => {
    const on = ref(false);
    const page = mount(
      host(() =>
        h(WSwitch, {
          modelValue: on.value,
          'onUpdate:modelValue': (v: boolean) => (on.value = v),
          label: 'Keep in tray',
        }),
      ),
      { attachTo: document.body },
    );
    const button = page.get('button[role=switch]');
    expect(button.attributes('aria-label')).toBe('Keep in tray');
    expect(button.attributes('aria-checked')).toBe('false');
    await button.trigger('click');
    expect(on.value).toBe(true);
    await nextTick();
    expect(page.get('button[role=switch]').attributes('aria-checked')).toBe('true');
    page.unmount();
  });

  it('with a reason stays focusable and aria-disabled, and a click changes nothing', async () => {
    const on = ref(true);
    const page = mount(
      host(() =>
        h(WSwitch, {
          modelValue: on.value,
          'onUpdate:modelValue': (v: boolean) => (on.value = v),
          label: 'Routing',
          disabledReason: 'Switched off accounts take no work.',
        }),
      ),
      { attachTo: document.body },
    );
    const button = page.get('button[role=switch]');
    expect(button.attributes('aria-disabled')).toBe('true');
    expect(button.attributes('disabled')).toBeUndefined();
    expect(button.attributes('aria-checked')).toBe('true');
    (button.element as HTMLButtonElement).focus();
    expect(document.activeElement).toBe(button.element);
    await button.trigger('click');
    await button.trigger('keydown', { key: 'Enter' });
    await button.trigger('keydown', { key: ' ' });
    expect(on.value).toBe(true);
    page.unmount();
  });
});

describe('SegmentedControl', () => {
  it('is a named radio group with the current choice checked; a click picks another', async () => {
    const theme = ref('light');
    const page = mount(
      host(() =>
        h(SegmentedControl, {
          modelValue: theme.value,
          'onUpdate:modelValue': (v: string) => (theme.value = v),
          label: 'Theme',
          options: [
            { value: 'light', label: 'Light' },
            { value: 'dark', label: 'Dark' },
          ],
        }),
      ),
      { attachTo: document.body },
    );
    expect(page.get('[role=radiogroup]').attributes('aria-label')).toBe('Theme');
    const radios = page.findAll('[role=radio]');
    expect(radios.map((r) => r.attributes('aria-checked'))).toEqual(['true', 'false']);
    await radios[1]!.trigger('click');
    expect(theme.value).toBe('dark');
    await nextTick();
    expect(page.findAll('[role=radio]').map((r) => r.attributes('aria-checked'))).toEqual(['false', 'true']);
    page.unmount();
  });
});

describe('chips', () => {
  it('ToggleChip says whether it is on through aria-pressed, in both looks', () => {
    for (const look of ['filled', 'outline'] as const) {
      expect(
        mount(ToggleChip, { props: { pressed: true, look } })
          .get('button')
          .attributes('aria-pressed'),
      ).toBe('true');
      expect(
        mount(ToggleChip, { props: { pressed: false, look } })
          .get('button')
          .attributes('aria-pressed'),
      ).toBe('false');
    }
  });

  it('RemovableChip is a button named by its action, in both sizes', async () => {
    for (const size of ['md', 'sm'] as const) {
      const chip = mount(RemovableChip, { props: { label: 'Remove builds', size }, slots: { default: 'builds' } });
      const button = chip.get('button');
      expect(button.attributes('type')).toBe('button');
      expect(button.attributes('aria-label')).toBe('Remove builds');
      expect(button.text()).toContain('builds');
    }
  });
});

describe('UsageBar', () => {
  it('is hidden from screen readers, colours by level or grey while idle, and marks a point when asked', () => {
    const bar = mount(UsageBar, { props: { percent: 40, level: 'mid' } });
    expect(bar.attributes('aria-hidden')).toBe('true');
    expect(bar.find('.bg-bar-mid').attributes('style')).toContain('width: 40%');
    expect(bar.find('[data-mark]').exists()).toBe(false);
    const idle = mount(UsageBar, { props: { percent: 40, level: 'idle', track: 'win' } });
    expect(idle.find('.bg-bar-idle').exists()).toBe(true);
    expect(idle.find('.bg-track-win').exists()).toBe(true);
    const marked = mount(UsageBar, { props: { percent: 40, level: 'low', mark: 98 } });
    expect(marked.get('[data-mark]').attributes('style')).toContain('left: 98%');
  });
});

describe('WButton', () => {
  it('is a plain button by default, a submit button when asked, and busy only while busy', () => {
    const plain = mount(WButton, { slots: { default: 'Save' } }).get('button');
    expect(plain.attributes('type')).toBe('button');
    expect(plain.attributes('aria-busy')).toBeUndefined();

    const busy = mount(WButton, { props: { type: 'submit', busy: true, icon: Folder }, slots: { default: 'Scan' } });
    expect(busy.get('button').attributes('type')).toBe('submit');
    expect(busy.get('button').attributes('aria-busy')).toBe('true');
    expect(busy.get('svg').attributes('aria-hidden')).toBe('true');
    expect(busy.get('svg').classes()).toContain('animate-spin-slow');
  });
});

describe('WIconButton', () => {
  it('is named by its label; with a reason it stays focusable and aria-disabled, and a click does nothing', async () => {
    let clicks = 0;
    const onClick = () => clicks++;
    const live = mount(WIconButton, { props: { label: 'Next day', icon: Folder }, attrs: { onClick } });
    expect(live.get('button').attributes('aria-label')).toBe('Next day');
    await live.get('button').trigger('click');
    expect(clicks).toBe(1);
    const off = mount(WIconButton, {
      props: { label: 'Next day', icon: Folder, disabledReason: 'This is today.' },
      attrs: { onClick },
    });
    const button = off.get('button');
    expect(button.attributes('aria-disabled')).toBe('true');
    expect(button.attributes('disabled')).toBeUndefined();
    expect(button.attributes('aria-label')).toBe('Next day');
    expect(button.classes().some((c) => c.startsWith('hov:'))).toBe(false);
    await button.trigger('click');
    expect(clicks).toBe(1);
  });
});

describe('NavItem', () => {
  it('is a link with href and a button without, marked aria-current only when current', () => {
    const link = mount(NavItem, {
      props: { icon: Folder, href: '#/settings', current: true },
      slots: { default: 'Settings' },
    });
    expect(link.element.tagName).toBe('A');
    expect(link.attributes('href')).toBe('#/settings');
    expect(link.attributes('type')).toBeUndefined();
    expect(link.attributes('aria-current')).toBe('page');

    const button = mount(NavItem, { props: { icon: Folder }, slots: { default: 'Overview' } });
    expect(button.element.tagName).toBe('BUTTON');
    expect(button.attributes('type')).toBe('button');
    expect(button.attributes('aria-current')).toBeUndefined();
  });
});

describe('WListItem', () => {
  it('with action is a button that emits select; plain, it has no button', async () => {
    const row = mount(WListItem, { props: { action: true }, slots: { default: 'Work' } });
    await row.get('button').trigger('click');
    expect(row.emitted('select')).toHaveLength(1);
    expect(
      mount(WListItem, { slots: { default: 'Work' } })
        .find('button')
        .exists(),
    ).toBe(false);
  });
});

describe('WTextField', () => {
  it('search has a hidden label naming the field, and typing updates the model', async () => {
    const text = ref('');
    const page = mount(
      host(() =>
        h(WTextField, {
          modelValue: text.value,
          'onUpdate:modelValue': (v: string) => (text.value = v),
          kind: 'search',
          label: 'Search sessions',
          placeholder: 'Search',
        }),
      ),
    );
    expect(page.get('label').text()).toBe('Search sessions');
    const input = page.get('input');
    expect(input.attributes('type')).toBe('search');
    expect(input.attributes('placeholder')).toBe('Search');
    await input.setValue('warden');
    expect(text.value).toBe('warden');
  });

  it('a path turns spelling checks off; a name leaves them alone', () => {
    expect(
      mount(WTextField, { props: { modelValue: '', kind: 'path' } })
        .get('input')
        .attributes('spellcheck'),
    ).toBe('false');
    expect(
      mount(WTextField, { props: { modelValue: '' } })
        .get('input')
        .attributes('spellcheck'),
    ).toBeUndefined();
  });
});

describe('AppModal', () => {
  const modal = (open: { value: boolean }) =>
    host(() => [
      h('button', { id: 'opener' }, 'Open'),
      h(
        AppModal,
        {
          open: open.value,
          'onUpdate:open': (v: boolean) => (open.value = v),
          title: 'Add account',
          description: 'Pick a folder',
        },
        () => h('div', { class: 'card' }, [h('button', { id: 'inside' }, 'Save')]),
      ),
    ]);

  it('names the dialog by its title and description, and Escape closes it', async () => {
    const open = ref(true);
    mount(modal(open), { attachTo: document.body });
    await flushPromises();
    const dialog = document.querySelector('[role=dialog]')!;
    expect(dialog).not.toBeNull();
    const title = document.getElementById(dialog.getAttribute('aria-labelledby')!);
    expect(title?.textContent).toBe('Add account');
    expect(document.getElementById(dialog.getAttribute('aria-describedby')!)?.textContent).toBe('Pick a folder');
    dialog.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await flushPromises();
    expect(open.value).toBe(false);
  });

  it('a click beside the card closes it; a click on the card does not', async () => {
    const open = ref(true);
    mount(modal(open), { attachTo: document.body });
    await flushPromises();
    (document.querySelector('.card') as HTMLElement).click();
    await flushPromises();
    expect(open.value).toBe(true);
    (document.querySelector('[role=dialog]') as HTMLElement).click();
    await flushPromises();
    expect(open.value).toBe(false);
  });

  it('with escapeOnly, a click beside the card leaves it open; Escape still closes it', async () => {
    const open = ref(true);
    mount(
      host(() =>
        h(
          AppModal,
          { open: open.value, 'onUpdate:open': (v: boolean) => (open.value = v), title: 'Set up', escapeOnly: true },
          () => h('div', { class: 'card' }, 'Card'),
        ),
      ),
      { attachTo: document.body },
    );
    await flushPromises();
    const dialog = document.querySelector('[role=dialog]') as HTMLElement;
    dialog.click();
    await flushPromises();
    expect(open.value).toBe(true);
    dialog.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await flushPromises();
    expect(open.value).toBe(false);
  });
});
