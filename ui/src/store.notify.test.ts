// Browser notifications: one when an account's folder is newly signed in as someone else, none in the desktop app,
// none without permission, and none again for a change already shown.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Account, State } from './api';
import { apply, store } from './store';
import { account, state } from './test/state';

const shown: { title: string; options: NotificationOptions }[] = [];
let permission: NotificationPermission = 'granted';

beforeEach(() => {
  shown.length = 0;
  permission = 'granted';
  vi.stubGlobal(
    'Notification',
    class {
      static get permission() {
        return permission;
      }
      constructor(title: string, options: NotificationOptions) {
        shown.push({ title, options });
      }
    },
  );
});

afterEach(() => {
  store.state = null;
  vi.unstubAllGlobals();
});

const MAIN = account({ id: 'main', label: 'Main' });
const changed = (email: string | null): Partial<Account> => ({
  identityChange: { now: email ? { email, organization: null } : null, knownAccountId: null },
});

function withAlt(over: Partial<Account> = {}, desktop = false): State {
  const s = state([MAIN, account({ id: 'a 1', label: 'Alt', ...over })]);
  return desktop ? { ...s, desktop: { startWithWindows: false } } : s;
}

describe('notifications on a changed sign-in', () => {
  it('shows one when an account is newly signed in as someone else, tagged by account', () => {
    apply(withAlt());
    apply(withAlt(changed('other@example.com')));
    expect(shown).toEqual([
      {
        title: 'Alt is signed in as someone else',
        options: {
          body: 'Its folder is now signed in as other@example.com. Warden keeps its usage apart until you confirm in Settings, Accounts.',
          tag: 'identity-a 1',
        },
      },
    ]);
  });

  it('names nobody when the folder is signed out', () => {
    apply(withAlt());
    apply(withAlt(changed(null)));
    expect(shown[0]!.options.body).toContain('signed in as nobody (signed out).');
  });

  it('does not repeat a change already shown, nor show one present in the first state', () => {
    apply(withAlt(changed('other@example.com')));
    apply(withAlt(changed('other@example.com')));
    expect(shown).toEqual([]);
  });

  it('stays quiet in the desktop app, which notifies from its core', () => {
    apply(withAlt({}, true));
    apply(withAlt(changed('other@example.com'), true));
    expect(shown).toEqual([]);
  });

  it('stays quiet without permission', () => {
    permission = 'default';
    apply(withAlt());
    apply(withAlt(changed('other@example.com')));
    permission = 'denied';
    apply(withAlt());
    apply(withAlt(changed('other@example.com')));
    expect(shown).toEqual([]);
  });

  it('shows it again after the change was settled and the sign-in changes once more', () => {
    apply(withAlt());
    apply(withAlt(changed('other@example.com')));
    apply(withAlt());
    apply(withAlt(changed('third@example.com')));
    expect(shown.map((n) => n.options.body)).toEqual([
      expect.stringContaining('other@example.com'),
      expect.stringContaining('third@example.com'),
    ]);
  });
});
