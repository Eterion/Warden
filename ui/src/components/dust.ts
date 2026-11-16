// A row leaving as dust, once its limit reset is used: it breaks into many small fragments that drift up and to the
// side, fading and shrinking, swept away from its left edge to its right at slightly different times (about 1.3 s in
// all), then the row closes its gap. The one animation besides the modals, menus and spinners. CSS and the Web
// Animations API only, styles set from script (which the Content-Security-Policy allows); the sizes come from the row
// itself. With reduced motion the row only fades out, quickly. Nothing keeps running after it: every animation is finite
// and the fragments are removed when they finish.

/** How many fragments high the row breaks into; their width matches. */
const ROWS_OF_FRAGMENTS = 6;
/** How long the sweep takes to reach the right edge, and how much later any one fragment may set off. */
const SWEEP_MS = 420;
const SCATTER_MS = 220;
/** How long one fragment drifts, at least, and up to how much longer. */
const DRIFT_MS = 600;
const DRIFT_SPREAD_MS = 320;
const CLOSE_MS = 260;
const FADE_MS = 150;

/** The row's own content: what the fragments are made of, and what is hidden while they drift. */
function contentOf(row: HTMLElement): HTMLElement {
  return row.firstElementChild instanceof HTMLElement ? row.firstElementChild : row;
}

/** Waits for an animation, which ends early (and rejects) when it is cancelled. */
async function ended(animation: Animation): Promise<void> {
  await animation.finished.catch(() => undefined);
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
      row.animate([{ opacity: 1 }, { opacity: 0 }], { duration: FADE_MS, easing: 'ease-out', fill: 'forwards' }),
    );
    return;
  }
  const content = contentOf(row);
  const box = content.getBoundingClientRect();
  if (box.width > 0 && box.height > 0) {
    const style = getComputedStyle(row);
    const layer = document.createElement('div');
    layer.setAttribute('aria-hidden', 'true');
    layer.inert = true;
    layer.className = 'pointer-events-none fixed z-1';
    Object.assign(layer.style, {
      left: `${box.left}px`,
      top: `${box.top}px`,
      width: `${box.width}px`,
      height: `${box.height}px`,
      color: style.color,
    });
    const grain = box.height / ROWS_OF_FRAGMENTS;
    const cols = Math.ceil(box.width / grain);
    const drifts: Animation[] = [];
    for (let r = 0; r < ROWS_OF_FRAGMENTS; r++) {
      for (let c = 0; c < cols; c++) {
        // A window onto one fragment of a copy of the row.
        const fragment = document.createElement('div');
        fragment.className = 'absolute overflow-hidden';
        Object.assign(fragment.style, {
          left: `${c * grain}px`,
          top: `${r * grain}px`,
          width: `${Math.min(grain, box.width - c * grain)}px`,
          height: `${grain}px`,
        });
        const copy = content.cloneNode(true) as HTMLElement;
        copy.removeAttribute('id');
        copy.querySelectorAll('[id]').forEach((e) => e.removeAttribute('id'));
        Object.assign(copy.style, {
          position: 'absolute',
          left: `${-c * grain}px`,
          top: `${-r * grain}px`,
          width: `${box.width}px`,
          height: `${box.height}px`,
          margin: '0',
        });
        fragment.append(copy);
        layer.append(fragment);
        // Swept from left to right, a little scattered; each drifts up and to the right on a gentle curve, turning,
        // shrinking and fading as it goes.
        const along = c / cols;
        const dx = grain * (2 + Math.random() * 6 + along * 2);
        const dy = -grain * (1.5 + Math.random() * 4.5);
        const turn = (Math.random() - 0.5) * 90;
        drifts.push(
          fragment.animate(
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
              delay: along * SWEEP_MS + Math.random() * SCATTER_MS + (r / ROWS_OF_FRAGMENTS) * 40,
              easing: 'cubic-bezier(0.4, 0, 0.8, 0.6)',
              fill: 'both',
            },
          ),
        );
      }
    }
    document.body.append(layer);
    content.style.visibility = 'hidden';
    await Promise.all(drifts.map(ended));
    layer.remove();
  }
  // Then the list closes the gap.
  const height = row.getBoundingClientRect().height;
  const border = getComputedStyle(row).borderBottomWidth;
  row.style.overflow = 'hidden';
  await ended(
    row.animate(
      [
        { height: `${height}px`, borderBottomWidth: border },
        { height: '0', borderBottomWidth: '0' },
      ],
      { duration: CLOSE_MS, easing: 'cubic-bezier(0.2, 0, 0, 1)', fill: 'forwards' },
    ),
  );
}

/** Brings a row back as it was, for a list that kept it (it now shows another of the same resets). */
export function undoDust(row: HTMLElement): void {
  delete row.dataset.gone;
  for (const a of row.getAnimations?.() ?? []) a.cancel();
  row.style.removeProperty('overflow');
  contentOf(row).style.removeProperty('visibility');
}

/** Whether `row` is turning to dust, or has and stays collapsed until its list takes it away or {@link undoDust}. */
export function isGone(row: HTMLElement): boolean {
  return row.dataset.gone !== undefined;
}
