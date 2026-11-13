// A stand-in for the local server in component tests: records each request and answers with a state.
import { vi } from 'vitest';
import type { State } from '../api';

export interface Call {
  method: string;
  path: string;
  body: unknown;
}

/** Replaces fetch; every request is recorded and answered with `answer()` (the current state by default). */
export function fakeServer(answer: () => State | { status: number; error: string }): Call[] {
  const calls: Call[] = [];
  vi.stubGlobal('fetch', async (path: string, init: RequestInit = {}) => {
    calls.push({
      method: init.method ?? 'GET',
      path,
      body: typeof init.body === 'string' ? JSON.parse(init.body) : undefined,
    });
    const a = answer();
    const failed = 'error' in a;
    return new Response(JSON.stringify(failed ? { error: a.error } : a), {
      status: failed ? a.status : 200,
      headers: { 'Content-Type': 'application/json' },
    });
  });
  return calls;
}
