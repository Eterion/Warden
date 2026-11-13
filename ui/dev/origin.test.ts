import { describe, expect, it } from 'vitest';
import { forwardedOrigin } from './origin';

const SERVER = 'http://127.0.0.1:4777';

describe('forwardedOrigin', () => {
  it('passes the UI on the dev server on as the server itself', () => {
    expect(forwardedOrigin('http://127.0.0.1:5173', '127.0.0.1:5173', SERVER)).toBe(SERVER);
  });

  it('forwards any other Origin unchanged, so the server refuses it', () => {
    expect(forwardedOrigin('https://example.com', '127.0.0.1:5173', SERVER)).toBe('https://example.com');
    expect(forwardedOrigin('http://127.0.0.1:9999', '127.0.0.1:5173', SERVER)).toBe('http://127.0.0.1:9999');
    expect(forwardedOrigin('null', '127.0.0.1:5173', SERVER)).toBe('null');
  });

  it('leaves a request without Origin without one', () => {
    expect(forwardedOrigin(undefined, '127.0.0.1:5173', SERVER)).toBeUndefined();
  });
});
