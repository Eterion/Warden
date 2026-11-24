import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import WList from './WList.vue';
import WTable from './WTable.vue';
import WWidget from './WWidget.vue';

describe('WWidget', () => {
  it('names the section with its h2 and places the controls and body inside one frame', () => {
    const widget = mount(WWidget, {
      props: { title: 'Limit resets', headingId: 'resets', focusableHeading: true },
      slots: {
        'title-after': '<span>2</span>',
        header: '<button>Filter</button>',
        default: '<p>Reset list</p>',
      },
    });
    expect(widget.element.tagName).toBe('SECTION');
    const heading = widget.get('header > h2');
    expect(widget.attributes('aria-labelledby')).toBe(heading.attributes('id'));
    expect(heading.text()).toBe('Limit resets 2');
    expect(heading.attributes('tabindex')).toBe('-1');
    const frame = widget.get('.border-divider-strong');
    expect(frame.classes()).toContain('rounded-lg');
    expect(frame.get('header > button').text()).toBe('Filter');
    expect(frame.get('p').text()).toBe('Reset list');
    expect(frame.find('a').exists()).toBe(false);
  });

  it('gives a footer its own full-width divider and equal padding below the body', () => {
    const widget = mount(WWidget, {
      props: { title: 'Memory' },
      slots: { body: '<p>Chart</p>', footer: '<p>Peak</p>' },
    });
    const body = widget.get('header + div');
    const footer = widget.get('.border-t');
    expect(body.text()).toBe('Chart');
    expect(body.classes()).not.toContain('pb-4');
    expect(body.element.nextElementSibling).toBe(footer.element);
    expect(footer.text()).toBe('Peak');
    expect(footer.classes()).toEqual(['border-t', 'border-divider', 'px-4', 'py-3']);
  });

  it('can hold tables and lists without a second frame', () => {
    const table = mount(WTable, { props: { cols: 'grid-cols-memory', on: 'panel', bare: true } });
    expect(table.classes()).toContain('overflow-x-auto');
    expect(table.classes()).not.toContain('border');
    expect(table.classes()).not.toContain('rounded-lg');
    expect(table.get('[role=row]').classes()).toContain('border-b');

    const list = mount(WList, { props: { bare: true } });
    expect(list.classes()).not.toContain('border');
    expect(list.classes()).not.toContain('rounded-lg');
    expect(mount(WList).classes()).toContain('border-divider-strong');
    expect(mount(WTable, { props: { cols: 'grid-cols-memory' } }).classes()).toContain('border-divider-strong');
  });
});
