import type { InjectionKey } from 'vue';

/** The grid template and gap a WTable gives its header and rows. */
export const TABLE_COLS: InjectionKey<() => string> = Symbol('table-cols');
