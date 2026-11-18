import { afterEach, describe, expect, it } from 'vitest';
import { ART, pickArt, turnToDust, undoDust } from './dust';

describe('turnToDust', () => {
  const restore: (() => void)[] = [];
  afterEach(() => {
    for (const r of restore.splice(0)) r();
    document.body.replaceChildren();
  });

  /** Stands in for the Web Animations API and layout: records each animation, which ends when the test says so. */
  function stub(): { el: Element; keyframes: Keyframe[]; timing: KeyframeAnimationOptions; done: () => void }[] {
    const calls: { el: Element; keyframes: Keyframe[]; timing: KeyframeAnimationOptions; done: () => void }[] = [];
    const proto = HTMLElement.prototype as unknown as Record<string, unknown>;
    const box = { left: 0, top: 0, width: 40, height: 20, right: 40, bottom: 20, x: 0, y: 0, toJSON: () => ({}) };
    for (const [name, value] of Object.entries({
      animate(this: Element, keyframes: Keyframe[], timing: KeyframeAnimationOptions) {
        let done: () => void = () => {};
        const finished = new Promise<void>((r) => (done = r));
        calls.push({ el: this, keyframes, timing, done });
        return { finished, cancel() {}, finish: () => done() };
      },
      getBoundingClientRect: () => box,
    })) {
      const had = Object.getOwnPropertyDescriptor(proto, name);
      Object.defineProperty(proto, name, { value, configurable: true, writable: true });
      restore.push(() => (had ? Object.defineProperty(proto, name, had) : delete proto[name]));
    }
    return calls;
  }

  function row(): HTMLElement {
    const li = document.createElement('li');
    li.append(document.createElement('div'));
    document.body.append(li);
    return li;
  }

  const flush = () => new Promise((r) => setTimeout(r, 0));
  const closes = (calls: ReturnType<typeof stub>, el: Element) =>
    calls.filter((c) => c.el === el && c.keyframes.some((k) => 'height' in k));

  it('picks one of the known characters for the hole, and undoDust takes it away again', async () => {
    expect(pickArt(0)).toBe('bunnies');
    expect(pickArt(0.49)).toBe('bunnies');
    expect(pickArt(0.5)).toBe('cthulhu');
    expect(pickArt(0.999)).toBe('cthulhu');
    for (let i = 0; i < 50; i++) expect(ART).toContain(pickArt());

    const calls = stub();
    const real = Math.random;
    Math.random = () => 0.99;
    restore.push(() => (Math.random = real));
    const r = row();
    void turnToDust(r, false);
    await flush();
    expect([...r.classList]).toEqual(['dust-hole', 'dust-art-cthulhu']);
    undoDust(r);
    expect(r.classList).toHaveLength(0);
    // Let the dust blow away and the gap close, so nothing is left in flight for the next test.
    for (const c of calls) c.done();
    await flush();
    for (const c of calls) c.done();
    await flush();
  });

  it('shows no dust while the window is out of sight, and ends it when the window goes away or is resized', async () => {
    const calls = stub();
    // Away from the start: no fragments, and the gap closes at once.
    document.documentElement.setAttribute('data-away', '');
    restore.push(() => document.documentElement.removeAttribute('data-away'));
    const a = row();
    void turnToDust(a, false);
    await flush();
    expect(document.querySelectorAll('[aria-hidden=true]')).toHaveLength(0);
    expect(closes(calls, a).map((c) => c.timing.duration)).toEqual([0]);
    for (const c of calls) c.done();
    await flush();
    document.documentElement.removeAttribute('data-away');

    // In sight, then away: the dust ends at once and the gap closes at once.
    const b = row();
    void turnToDust(b, false);
    await flush();
    const drifts = calls.filter((c) => c.el !== a && c.el !== b);
    expect(drifts.length).toBeGreaterThan(0);
    document.documentElement.setAttribute('data-away', '');
    await flush();
    expect(document.querySelectorAll('[aria-hidden=true]')).toHaveLength(0);
    expect(closes(calls, b).map((c) => c.timing.duration)).toEqual([0]);
    for (const c of calls) c.done();
    await flush();
    document.documentElement.removeAttribute('data-away');

    // In sight, then resized: the dust ends at once, and the gap closes as usual.
    const d = row();
    void turnToDust(d, false);
    await flush();
    window.dispatchEvent(new Event('resize'));
    await flush();
    expect(document.querySelectorAll('[aria-hidden=true]')).toHaveLength(0);
    expect(closes(calls, d).map((c) => c.timing.duration)).toEqual([320]);
    for (const c of calls) c.done();
    await flush();
  });

  it('holds a row’s gap until no other row is still turning to dust, so nothing moves under the dust', async () => {
    const calls = stub();
    const a = row();
    const b = row();
    void turnToDust(a, false);
    await flush();
    const ofA = calls.length;
    expect(ofA).toBeGreaterThan(0);
    void turnToDust(b, false);
    await flush();
    const ofB = calls.slice(ofA);
    expect(ofB.length).toBeGreaterThan(0);

    // The first row's dust has blown away while the second's still drifts: its gap waits.
    for (const c of calls.slice(0, ofA)) c.done();
    await flush();
    expect(closes(calls, a)).toHaveLength(0);
    expect(a.classList).toContain('dust-hole');

    // Once the second's has too, both gaps close.
    for (const c of ofB) c.done();
    await flush();
    expect(closes(calls, a)).toHaveLength(1);
    expect(closes(calls, b)).toHaveLength(1);
    for (const c of [...closes(calls, a), ...closes(calls, b)]) c.done();
    await flush();
  });
});
