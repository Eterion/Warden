import { afterEach, describe, expect, it, vi } from 'vitest';
import { api, apiUrl } from './api';
import { fakeServer } from './test/api';
import { state } from './test/state';

describe('the API transport', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('is the page’s own server in a browser', async () => {
    expect(apiUrl('/api/state')).toBe('/api/state');
    const calls = fakeServer(() => state([]));
    await api.state();
    expect(calls.map((c) => c.path)).toEqual(['/api/state']);
  });

  it('is the app’s own scheme inside the desktop app', async () => {
    vi.stubGlobal('isTauri', true);
    expect(apiUrl('/api/state')).toBe('http://warden.localhost/api/state');
    const calls = fakeServer(() => state([]));
    await api.refresh();
    expect(calls.map((c) => c.path)).toEqual(['http://warden.localhost/api/usage/refresh']);
  });
});
