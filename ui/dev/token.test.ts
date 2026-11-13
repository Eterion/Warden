import { describe, expect, it } from 'vitest';
import { tradeToken } from './token';

describe('tradeToken', () => {
  it('puts the token into the server’s cookie and out of the address', () => {
    expect(tradeToken('/?token=ab12', 4777)).toEqual({
      location: '/',
      setCookie: 'warden-token-4777=ab12; Path=/; HttpOnly; SameSite=Strict',
    });
    expect(tradeToken('/settings?x=1&token=ab12', 4800)?.location).toBe('/settings?x=1');
  });

  it('leaves other addresses alone', () => {
    expect(tradeToken('/', 4777)).toBeUndefined();
    expect(tradeToken('/?token=', 4777)).toBeUndefined();
    expect(tradeToken('/?token=not%3Bhex', 4777)).toBeUndefined();
    expect(tradeToken('/api/state?token=ab12', 4777)).toBeUndefined();
  });
});
