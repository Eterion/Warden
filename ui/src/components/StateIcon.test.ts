import { mount } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import StateIcon from './StateIcon.vue';

afterEach(() => {
  vi.restoreAllMocks();
  delete (Element.prototype as Partial<Element>).getAnimations;
});

describe('StateIcon', () => {
  it('shows done as a dash in the muted colour, not a check or a flag', () => {
    const svg = mount(StateIcon, { props: { state: 'done' } }).get('svg');
    expect(svg.classes()).toContain('lucide-minus-icon');
    expect(svg.classes()).not.toContain('lucide-flag-icon');
    expect(svg.classes()).not.toContain('lucide-check-icon');
    expect(svg.classes()).not.toContain('lucide-circle-check-icon');
    expect(svg.classes()).toContain('text-icon-faded');
    expect(svg.classes()).toContain('size-4');
    expect(svg.attributes('aria-hidden')).toBe('true');
  });

  it('shows done at the small size too', () => {
    const svg = mount(StateIcon, { props: { state: 'done', size: 'sm' } }).get('svg');
    expect(svg.classes()).toContain('lucide-minus-icon');
    expect(svg.classes()).toContain('text-icon-faded');
    expect(svg.classes()).toContain('size-3.25');
  });

  it('shows a state that is not known as a ghost in the muted colour, not a dashed circle', () => {
    const svg = mount(StateIcon, { props: { state: 'unknown' } }).get('svg');
    expect(svg.classes()).toContain('lucide-ghost-icon');
    expect(svg.classes()).not.toContain('lucide-circle-dashed-icon');
    expect(svg.classes()).toContain('text-icon-faded');
    expect(svg.classes()).not.toContain('text-icon');
    expect(svg.classes()).toContain('size-4');
    expect(svg.attributes('aria-hidden')).toBe('true');
  });

  it('shows a state that is not known at the small size too', () => {
    const svg = mount(StateIcon, { props: { state: 'unknown', size: 'sm' } }).get('svg');
    expect(svg.classes()).toContain('lucide-ghost-icon');
    expect(svg.classes()).toContain('text-icon-faded');
    expect(svg.classes()).toContain('size-3.25');
  });

  it('draws working as a still thin ring with the darker arc turning on it, only the arc on the shared clock', () => {
    // Record which elements v-sync-spin puts on the shared clock.
    const synced: Element[] = [];
    Element.prototype.getAnimations = function (this: Element) {
      synced.push(this);
      return [];
    };
    const wrapper = mount(StateIcon, { props: { state: 'working', size: 'sm' } });
    const ring = wrapper.get('span');
    expect(ring.attributes('aria-hidden')).toBe('true');
    expect(ring.classes()).toContain('size-3.25');

    const [track, arc] = wrapper.findAll('svg');
    expect(track!.attributes('data-ring-track')).toBeDefined();
    expect(track!.classes()).toEqual(expect.arrayContaining(['lucide-circle-icon', 'scale-90', 'text-track-win']));
    expect(track!.classes()).not.toContain('animate-spin-slow');
    expect(arc!.classes()).toEqual(
      expect.arrayContaining(['lucide-loader-circle-icon', 'animate-spin-slow', 'text-icon']),
    );
    // The track's line stays thinner than the arc's, even before its 90% scale.
    expect(Number(track!.attributes('stroke-width'))).toBeLessThan(Number(arc!.attributes('stroke-width')));

    expect(synced).toEqual([arc!.element]);
  });
});
