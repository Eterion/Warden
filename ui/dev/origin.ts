// The Origin the development proxy forwards to the local server. The server accepts only its own Origin, so a
// request from the UI on Vite's dev server (same Host as the page) is passed on as the server's own. Any other
// Origin, such as another website's, is forwarded as it came, and the server's Origin check refuses it.

export function forwardedOrigin(
  origin: string | undefined,
  host: string | undefined,
  server: string,
): string | undefined {
  if (origin === undefined || host === undefined) return origin;
  return origin === `http://${host}` ? server : origin;
}
