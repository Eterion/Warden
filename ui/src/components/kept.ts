// What a modal draws while it closes. Closing clears the state its content comes from (the address, the account being
// renamed), but the modal is still on screen for its closing animation; drawing from a kept copy shows what was there
// until it is gone, instead of what the closed state would give.
import { shallowRef, watchEffect, type Ref } from 'vue';

/** Follows `value` while `open` is true and keeps its last value while it is false. */
export function keptWhileClosing<T>(open: () => boolean, value: () => T): Readonly<Ref<T>> {
  const kept = shallowRef(value());
  watchEffect(() => {
    if (open()) kept.value = value();
  });
  return kept;
}
