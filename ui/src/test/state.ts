// Made-up accounts and state for component tests.
import type { Account, State } from '../api';
export const NOW = new Date(2026, 10, 9, 10, 0).getTime();
export function account(a: Partial<Account> & Pick<Account, 'id' | 'label'>): Account {
  return {
    provider: 'claude',
    folder: '%USERPROFILE%\\.claude-' + a.id,
    enabled: true,
    sameAsMain: false,
    identity: null,
    identityChange: null,
    usage: null,
    usageError: null,
    resets: [],
    hooks: 'installed',
    ...a,
  };
}
export function state(accounts: Account[]): State {
  const main = accounts.find((a) => a.id === 'main');
  return {
    hooksOff: false,
    version: '0.1.0',
    setupDone: true,
    dataDir: '%APPDATA%\\Warden',
    logDir: '%APPDATA%\\Warden\\logs',
    settings: {
      reserve: 25,
      usageIntervalMin: 5,
      theme: 'system',
      keepInTray: false,
      workHours: null,
      resetWarningDays: 3,
      showQuietSessions: true,
      hiddenResetAccounts: [],
      soonestResetOnly: false,
    },
    main: main ? { ...main, signIn: { state: 'signedIn', email: main.identity?.email ?? null } } : null,
    accounts: accounts.filter((a) => a.id !== 'main'),
    problems: [],
    lastRead: NOW - 60000,
    revision: 1,
    homeDir: '%USERPROFILE%',
    delegation: {
      mainWorksItself: true,
      delegated: accounts.find((a) => a.id !== 'main' && a.enabled)
        ? { accountId: accounts.find((a) => a.id !== 'main' && a.enabled)!.id }
        : null,
      mainReason: 'Main has room.',
      delegatedReason: 'Most allowance left.',
      handOffAt: 75,
    },
    routeTool: '%APPDATA%\\Warden\\warden.exe',
    sessions: [],
    activity: [],
    desktop: null,
    update: null,
  };
}
