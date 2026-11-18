// A row leaving as dust, once its limit reset is used: the whole row, its background too, breaks into many small
// fragments that drift up and to the side, fading and shrinking, swept away from its left edge to its right at slightly
// different times (about 2.5 s in all), uncovering one of the characters where the row was, picked at random (three dust bunnies, one waving goodbye, or
// Cthulhu rising from below); then the gap closes up. The one animation besides the modals, menus and spinners. CSS and the Web
// Animations API only, styles set from script (which the Content-Security-Policy allows); the sizes come from the row
// itself. With reduced motion the row only fades out, quickly. Nothing keeps running after it: every animation is finite
// and the fragments are removed when they finish.

/** How many fragments high the row breaks into (fine dust, a few pixels each); their width matches. */
const ROWS_OF_FRAGMENTS = 10;
/** How far the fragments travel, in sixths of the row's height: the same whatever the dust's grain. */
const TRAVEL_PARTS = 6;
/**
 * How long the sweep takes to reach the right edge, and how much later any one fragment may set off. With the stagger
 * and the longest drift, the last fragment is gone by {@link DUST_MS}.
 */
const SWEEP_MS = 700;
const SCATTER_MS = 300;
/** How long one fragment drifts, at least, and up to how much longer. */
const DRIFT_MS = 1000;
const DRIFT_SPREAD_MS = 450;
/** How much later the bottom fragment row sets off than the top one. */
const ROW_STAGGER_MS = 50;
/** The latest any fragment finishes: the dust's whole length, before the gap closes. */
export const DUST_MS = SWEEP_MS + SCATTER_MS + ROW_STAGGER_MS + DRIFT_MS + DRIFT_SPREAD_MS;
const CLOSE_MS = 320;
const FADE_MS = 150;

/** The row's own content: what the fragments are made of, and what is hidden while they drift. */
function contentOf(row: HTMLElement): HTMLElement {
  return row.firstElementChild instanceof HTMLElement ? row.firstElementChild : row;
}

/** The colour the row is seen on: its own background, else the first one behind it (the list's, the panel's). */
function surfaceOf(row: HTMLElement): string {
  for (let e: Element | null = row; e; e = e.parentElement) {
    const colour = getComputedStyle(e).backgroundColor;
    if (colour && colour !== 'transparent' && !/^rgba\(.*,\s*0\)$/.test(colour)) return colour;
  }
  return 'transparent';
}

/**
 * The rounding of the list box's corners inside its border, for the row's corner fragments when the row is the list's
 * first or last: they start as the row looks, clipped by the box.
 */
function cornerOf(row: HTMLElement): { top: string; bottom: string } {
  const list = row.parentElement;
  if (!list) return { top: '0', bottom: '0' };
  const style = getComputedStyle(list);
  const inner = (radius: string, border: string) => `${Math.max(0, parseFloat(radius) - parseFloat(border)) || 0}px`;
  return {
    top: row === list.firstElementChild ? inner(style.borderTopLeftRadius, style.borderTopWidth) : '0',
    bottom: row === list.lastElementChild ? inner(style.borderBottomLeftRadius, style.borderBottomWidth) : '0',
  };
}

/**
 * Where the dust is drawn: a layer in the area that scrolls the row (the content panel), covering all it scrolls and
 * clipping to it, so the fragments move with the row if the panel scrolls and never spill past its edges; `at` is the
 * row's place in that layer. Without such an area (or one the layer cannot sit in), the window, as a fixed layer.
 */
function layerFor(
  row: HTMLElement,
  box: DOMRect,
): { layer: HTMLDivElement; parent: HTMLElement; at: { x: number; y: number } } {
  const layer = document.createElement('div');
  layer.setAttribute('aria-hidden', 'true');
  layer.inert = true;
  for (let e = row.parentElement; e && e !== document.body; e = e.parentElement) {
    const style = getComputedStyle(e);
    if (!/(auto|scroll)/.test(style.overflowY) || style.position === 'static') continue;
    const area = e.getBoundingClientRect();
    layer.className = 'pointer-events-none absolute top-0 left-0 z-1 overflow-hidden';
    Object.assign(layer.style, { width: `${e.clientWidth}px`, height: `${e.scrollHeight}px` });
    return {
      layer,
      parent: e,
      at: { x: box.left - area.left - e.clientLeft + e.scrollLeft, y: box.top - area.top - e.clientTop + e.scrollTop },
    };
  }
  layer.className = 'pointer-events-none fixed inset-0 z-1 overflow-hidden';
  return { layer, parent: document.body, at: { x: box.left, y: box.top } };
}

/**
 * Rows turning to dust now, and gaps closing now. A row's gap waits until no other row is still turning to dust, and new
 * dust waits for any gap still closing, so no row moves under another's dust while it drifts.
 */
const drifting = new Set<Promise<void>>();
const closing = new Set<Promise<void>>();

async function settled(set: Set<Promise<void>>): Promise<void> {
  while (set.size) await Promise.all([...set]);
}

/** The characters the hole can show, one picked at random each time. */
export const ART = ['bunnies', 'cthulhu'] as const;
/** Each one's utility in tokens.css, its class written out in full so the CSS build keeps it. */
const ART_CLASS: Record<(typeof ART)[number], string> = {
  bunnies: 'dust-art-bunnies',
  cthulhu: 'dust-art-cthulhu',
};

/** One of {@link ART}, at random. */
export function pickArt(roll = Math.random()): (typeof ART)[number] {
  return ART[Math.min(ART.length - 1, Math.floor(roll * ART.length))]!;
}

/** Waits for an animation, which ends early (and rejects) when it is cancelled. */
async function ended(animation: Animation): Promise<void> {
  await animation.finished.catch(() => undefined);
}

/**
 * Every dust animation running now. Nothing animates while the window is out of sight (hidden or minimized, `data-away`
 * on the root), and a resize can move the list away from where its dust was drawn: either ends them all at once, and
 * while the window is away, new ones take no time.
 */
const running = new Set<Animation>();
let watching = false;

function away(): boolean {
  return document.hidden || document.documentElement.hasAttribute('data-away');
}

function finishAll(): void {
  for (const a of [...running]) a.finish?.();
}

function watchSight(): void {
  if (watching) return;
  watching = true;
  document.addEventListener('visibilitychange', () => {
    if (away()) finishAll();
  });
  window.addEventListener('resize', finishAll);
  new MutationObserver(() => {
    if (away()) finishAll();
  }).observe(document.documentElement, { attributes: true, attributeFilter: ['data-away'] });
}

/** Starts an animation of the dust, kept track of until it ends (at once while the window is away). */
function play(el: Element, keyframes: Keyframe[], timing: KeyframeAnimationOptions): Animation {
  watchSight();
  const quick = away();
  const animation = el.animate(keyframes, quick ? { ...timing, duration: 0, delay: 0 } : timing);
  running.add(animation);
  void ended(animation).then(() => running.delete(animation));
  return animation;
}

/**
 * Turns `row` to dust and closes its gap (or, with `reduce`, only fades it out). Resolves once it is gone from sight; the
 * row stays in the page, collapsed, for its list to take away ({@link undoDust} brings it back if the list keeps it).
 * From the start it is marked gone ({@link isGone}), so nothing gives focus to what it holds.
 */
export async function turnToDust(row: HTMLElement, reduce: boolean): Promise<void> {
  row.dataset.gone = '';
  if (typeof row.animate !== 'function') return;
  if (reduce) {
    await ended(
      play(row, [{ opacity: 1 }, { opacity: 0 }], { duration: FADE_MS, easing: 'ease-out', fill: 'forwards' }),
    );
    return;
  }
  await settled(closing);
  // The character the hole shows this time: picked first, before the fragments' own randomness.
  const art = pickArt();
  const content = contentOf(row);
  const box = content.getBoundingClientRect();
  // Out of sight, no dust at all: the row only closes up, at once.
  if (box.width > 0 && box.height > 0 && !away()) {
    const { layer, parent, at } = layerFor(row, box);
    layer.style.color = getComputedStyle(row).color;
    const corner = cornerOf(row);
    // No fragment smaller than the list's corner rounding, so a corner fragment holds its whole curve; the columns all
    // one width, none left narrow at the right edge.
    const least = Math.max(1, parseFloat(corner.top) || 0, parseFloat(corner.bottom) || 0);
    const rows = Math.max(1, Math.min(ROWS_OF_FRAGMENTS, Math.floor(box.height / least)));
    const high = box.height / rows;
    const cols = Math.max(1, Math.floor(box.width / high));
    const wide = box.width / cols;
    const step = box.height / TRAVEL_PARTS;
    const surface = surfaceOf(row);
    const drifts: Animation[] = [];
    for (let r = 0; r < rows; r++) {
      for (let c = 0; c < cols; c++) {
        // A window onto one fragment of a copy of the row.
        const fragment = document.createElement('div');
        fragment.className = 'absolute overflow-hidden';
        Object.assign(fragment.style, {
          left: `${at.x + c * wide}px`,
          top: `${at.y + r * high}px`,
          width: `${wide}px`,
          height: `${high}px`,
          // The row's background goes with it, so the whole row turns to dust, not only its text.
          backgroundColor: surface,
        });
        const first = c === 0;
        const last = c === cols - 1;
        if (r === 0 && first) fragment.style.borderTopLeftRadius = corner.top;
        if (r === 0 && last) fragment.style.borderTopRightRadius = corner.top;
        if (r === rows - 1 && first) fragment.style.borderBottomLeftRadius = corner.bottom;
        if (r === rows - 1 && last) fragment.style.borderBottomRightRadius = corner.bottom;
        const copy = content.cloneNode(true) as HTMLElement;
        copy.removeAttribute('id');
        copy.querySelectorAll('[id]').forEach((e) => e.removeAttribute('id'));
        Object.assign(copy.style, {
          position: 'absolute',
          left: `${-c * wide}px`,
          top: `${-r * high}px`,
          width: `${box.width}px`,
          height: `${box.height}px`,
          margin: '0',
        });
        fragment.append(copy);
        layer.append(fragment);
        // Swept from left to right, a little scattered; each drifts up and to the right on a gentle curve, turning,
        // shrinking and fading as it goes.
        const along = c / cols;
        const dx = step * (2 + Math.random() * 6 + along * 2);
        const dy = -step * (1.5 + Math.random() * 4.5);
        const turn = (Math.random() - 0.5) * 90;
        drifts.push(
          play(
            fragment,
            [
              { transform: 'translate(0, 0) rotate(0deg) scale(1)', opacity: 1 },
              {
                offset: 0.4,
                transform: `translate(${dx * 0.3}px, ${dy * 0.5}px) rotate(${turn * 0.4}deg) scale(0.9)`,
                opacity: 0.85,
              },
              { transform: `translate(${dx}px, ${dy}px) rotate(${turn}deg) scale(0.3)`, opacity: 0 },
            ],
            {
              duration: DRIFT_MS + Math.random() * DRIFT_SPREAD_MS,
              delay: along * SWEEP_MS + Math.random() * SCATTER_MS + (r / rows) * ROW_STAGGER_MS,
              easing: 'cubic-bezier(0.4, 0, 0.8, 0.6)',
              fill: 'both',
            },
          ),
        );
      }
    }
    parent.append(layer);
    // Behind the dust, the hole the row leaves: uncovered as the fragments blow away.
    row.classList.add('dust-hole', ART_CLASS[art]);
    content.style.visibility = 'hidden';
    const drift = Promise.all(drifts.map(ended)).then(() => undefined);
    drifting.add(drift);
    await drift;
    drifting.delete(drift);
    layer.remove();
  }
  // Then the list closes the gap, once no other row is still turning to dust.
  await settled(drifting);
  const height = row.getBoundingClientRect().height;
  const border = getComputedStyle(row).borderBottomWidth;
  row.style.overflow = 'hidden';
  const close = ended(
    play(
      row,
      [
        { height: `${height}px`, borderBottomWidth: border },
        { height: '0', borderBottomWidth: '0' },
      ],
      { duration: CLOSE_MS, easing: 'cubic-bezier(0.2, 0, 0, 1)', fill: 'forwards' },
    ),
  );
  closing.add(close);
  await close;
  closing.delete(close);
}

/** Brings a row back as it was, for a list that kept it (it now shows another of the same resets). */
export function undoDust(row: HTMLElement): void {
  delete row.dataset.gone;
  row.classList.remove('dust-hole', ...Object.values(ART_CLASS));
  for (const a of row.getAnimations?.() ?? []) a.cancel();
  row.style.removeProperty('overflow');
  contentOf(row).style.removeProperty('visibility');
}

/** Whether `row` is turning to dust, or has and stays collapsed until its list takes it away or {@link undoDust}. */
export function isGone(row: HTMLElement): boolean {
  return row.dataset.gone !== undefined;
}
