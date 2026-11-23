import { mount } from '@vue/test-utils';
import { MemoryStick } from '@lucide/vue';
import { describe, expect, it } from 'vitest';
import OwlIcon from './OwlIcon.vue';

/** A drawn line's thickness on screen, in pixels: its stroke in the icon's units, scaled from its view to its size. */
function strokePx(svg: Element): number {
  const view = Number(svg.getAttribute('viewBox')!.split(' ')[2]);
  return (Number(svg.getAttribute('stroke-width')) * Number(svg.getAttribute('width'))) / view;
}

describe('OwlIcon', () => {
  it("draws its lines as thick as a Lucide icon's of the same size and stroke", () => {
    for (const [size, stroke] of [
      [16, 2],
      [24, 2],
      [32, 1.5],
      [48, 1],
    ] as const) {
      const owl = mount(OwlIcon, { props: { size, strokeWidth: stroke } }).get('svg').element;
      const lucide = mount(MemoryStick, { props: { size, strokeWidth: stroke } }).get('svg').element;
      expect(owl.getAttribute('width')).toBe(String(size));
      expect(strokePx(owl)).toBeCloseTo(strokePx(lucide), 6);
    }
  });

  it("is a line icon as Lucide's are: no fill, round caps and joins, in the text's colour", () => {
    const svg = mount(OwlIcon).get('svg');
    expect(svg.attributes()).toMatchObject({
      fill: 'none',
      stroke: 'currentColor',
      'stroke-linecap': 'round',
      'stroke-linejoin': 'round',
      width: '24',
    });
    // Only the outline: one path, nothing filled inside it.
    expect(svg.findAll('path')).toHaveLength(1);
    expect(svg.findAll('circle')).toHaveLength(0);
    expect(svg.findAll('path [fill], path[fill]')).toHaveLength(0);
  });
});
