// Where a popup (a tooltip or a menu) is drawn: into the modal its trigger sits in, if any, so it stays inside that
// dialog's landmark; otherwise into the page's main landmark it sits in, or the page body.
import { onMounted, ref, type Ref } from 'vue';

export function usePortalTarget(trigger: Ref<{ $el: Node | null } | null>): Ref<string | HTMLElement> {
  const to = ref<string | HTMLElement>('body');
  onMounted(() => {
    // $el can be a fragment's placeholder, so look from its parent.
    const from = trigger.value?.$el?.parentElement;
    const host = from?.closest('[role=dialog]') ?? from?.closest('main');
    if (host instanceof HTMLElement) to.value = host;
  });
  return to;
}
