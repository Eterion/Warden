import { describe, expect, it } from 'vitest';
import { onAnimationStart, pauseSpinners, sync } from './spin';

describe('v-sync-spin', () => {
  it('starts every animation on the element at the timeline’s zero', () => {
    const animations = [{ startTime: 1234 }, { startTime: 0 }];
    const el = { getAnimations: () => animations } as unknown as Element;
    sync(el);
    expect(animations.map((a) => a.startTime)).toEqual([0, 0]);
  });

  it('puts a spinner whose animation started again back on the shared clock', () => {
    const animations = [{ startTime: 99103 }];
    const el = document.createElement('span');
    el.getAnimations = () => animations as unknown as Animation[];
    onAnimationStart({ animationName: 'spin', target: el } as unknown as AnimationEvent);
    expect(animations[0]!.startTime).toBe(0);
    // Other animations (a modal opening) are left alone.
    animations[0]!.startTime = 50;
    onAnimationStart({ animationName: 'modal-in', target: el } as unknown as AnimationEvent);
    expect(animations[0]!.startTime).toBe(50);
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
