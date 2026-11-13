import { describe, expect, it } from 'vitest';
import { pauseSpinners, sync } from './spin';

describe('v-sync-spin', () => {
  it('starts every animation on the element at the timeline’s zero', () => {
    const animations = [{ startTime: 1234 }, { startTime: 0 }];
    const el = { getAnimations: () => animations } as unknown as Element;
    sync(el);
    expect(animations.map((a) => a.startTime)).toEqual([0, 0]);
  });

  it('leaves elements alone where animations cannot be read', () => {
    expect(() => sync({} as Element)).not.toThrow();
  });

  it('pauses spinners while away and puts them back on the shared clock after', () => {
    const spin = { animationName: 'spin', startTime: 500 as number | null };
    const other = { animationName: 'modal-in', startTime: 300 };
    const original = document.getAnimations;
    document.getAnimations = () => [spin, other] as unknown as Animation[];
    pauseSpinners(true);
    expect(document.documentElement.hasAttribute('data-away')).toBe(true);
    expect(spin.startTime).toBe(500);
    pauseSpinners(false);
    expect(document.documentElement.hasAttribute('data-away')).toBe(false);
    expect(spin.startTime).toBe(0);
    expect(other.startTime).toBe(300);
    document.getAnimations = original;
  });
});
