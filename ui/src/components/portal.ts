// Where a popup (a tooltip or a menu) is drawn: into the modal its trigger sits in, if any, so it stays inside that
// dialog's landmark. Otherwise, use the main, banner or sidebar landmark that contains the trigger, then the page body.
import { onMounted, ref, type Ref } from 'vue';

export function usePortalTarget(trigger: Ref<{ $el: Node | null } | null>): Ref<string | HTMLElement> {
  const to = ref<string | HTMLElement>('body');
  onMounted(() => {
    // $el can be a fragment's placeholder, so look from its parent.
    const from = trigger.value?.$el?.parentElement;
    const host = from?.closest('[role=dialog]') ?? from?.closest('main, header, aside');
    if (host instanceof HTMLElement) to.value = host;
  });
  return to;
}
