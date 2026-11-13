import assert from 'node:assert/strict';
import { test } from 'node:test';
import { browserCsp, desktopCsp } from './csp.ts';

test('both policies are found and allow only the app itself', () => {
  for (const csp of [browserCsp(), desktopCsp()]) {
    assert.match(csp, /default-src 'none'/);
    assert.match(csp, /script-src 'self';/);
    assert.doesNotMatch(csp, /unsafe-/);
  }
  assert.match(desktopCsp(), /connect-src 'self' ipc: http:\/\/ipc\.localhost$/);
});
