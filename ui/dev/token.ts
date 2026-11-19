// The demo server's token (docs/api.md). The demo prints an address carrying a random token for this run, and
// answers only requests with the token's cookie. Opened on Vite's dev server instead (the same address with
// Vite's port), the token is put into the server's cookie here and taken out of the address bar; the proxy then
// passes the cookie on with every /api/ request, since cookies do not depend on the port.

/** The cookie the demo server on this port expects. */
export function tokenCookie(serverPort: number): string {
  return `warden-token-${serverPort}`;
}

/**
 * For a page address carrying `?token=`: where to go without it and the cookie to set. `undefined` for any other
 * address.
 */
export function tradeToken(url: string, serverPort: number): { location: string; setCookie: string } | undefined {
  const parsed = new URL(url, 'http://127.0.0.1');
  const token = parsed.searchParams.get('token');
  if (!token || !/^[0-9a-f]+$/.test(token) || parsed.pathname.startsWith('/api/')) return undefined;
  parsed.searchParams.delete('token');
  return {
    location: parsed.pathname + parsed.search + parsed.hash,
    setCookie: `${tokenCookie(serverPort)}=${token}; Path=/; HttpOnly; SameSite=Strict`,
  };
}
