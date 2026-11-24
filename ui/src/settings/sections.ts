// The labels, icons and subtitles for the sections in Settings and the Warden menu.
import { AppWindow, CircleUserRound, Palette } from '@lucide/vue';
import type { Section } from '../route';

export const sections = {
  app: {
    label: 'App',
    icon: AppWindow,
    subtitle: "Usage reading, the reset warning, running in the background and Warden's own files.",
  },
  appearance: {
    label: 'Appearance',
    icon: Palette,
    subtitle: 'Light or dark, and how pages change.',
  },
  accounts: {
    label: 'Accounts',
    icon: CircleUserRound,
    subtitle: 'Main and the accounts that take delegated work.',
  },
} satisfies Record<Section, { label: string; icon: typeof AppWindow; subtitle: string }>;
