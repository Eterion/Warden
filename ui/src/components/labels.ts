// A label and its control react as one. `v-label-state` on a control (a switch, checkbox, radio or text field) finds
// its labels (a wrapping <label> or <label for>, or for a control HTML cannot label, such as a slider's thumb, the
// element its aria-labelledby names) and, while one is hovered or pressed, marks the control with data-label-hover or
// data-label-press, which the hov: and press: variants style like the control's own :hover and :active. Clicking an
// aria-labelledby label focuses its control, as clicking a <label> does.
import { useEventListener } from '@vueuse/core';
import type { ObjectDirective } from 'vue';

const stops = new WeakMap<HTMLElement, (() => void)[]>();

function labelsOf(el: HTMLElement): HTMLElement[] {
  const labels = (el as HTMLElement & { labels?: NodeListOf<HTMLLabelElement> | null }).labels;
  return labels ? [...labels] : [];
}

/** The elements a control's aria-labelledby names, looked up in the tree it sits in (attached or not yet). */
function labelledBy(el: HTMLElement): HTMLElement[] {
  const ids = el.getAttribute('aria-labelledby')?.split(/s+/).filter(Boolean) ?? [];
  const root = el.getRootNode() as ParentNode;
  return ids.flatMap((id) => {
    const found = root.querySelector?.<HTMLElement>(`#${CSS.escape(id)}`);
    return found ? [found] : [];
  });
}

function attach(el: HTMLElement): void {
  detach(el);
  const set = (name: string, on: boolean) => el.toggleAttribute(name, on);
  const list: (() => void)[] = [];
  for (const label of labelledBy(el)) list.push(useEventListener(label, 'click', () => el.focus()));
  for (const label of [...labelsOf(el), ...labelledBy(el)]) {
    list.push(
      useEventListener(label, 'pointerenter', () => set('data-label-hover', true)),
      useEventListener(label, 'pointerleave', () => {
        set('data-label-hover', false);
        set('data-label-press', false);
      }),
      useEventListener(label, 'pointerdown', (e: PointerEvent) => {
        if (e.button === 0) set('data-label-press', true);
      }),
      useEventListener(label, ['pointerup', 'pointercancel'], () => set('data-label-press', false)),
    );
  }
  stops.set(el, list);
}

function detach(el: HTMLElement): void {
  for (const stop of stops.get(el) ?? []) stop();
  stops.delete(el);
  el.removeAttribute('data-label-hover');
  el.removeAttribute('data-label-press');
}

export const vLabelState: ObjectDirective<HTMLElement> = { mounted: attach, beforeUnmount: detach };
