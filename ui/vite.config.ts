import { defineConfig, type Plugin } from 'vitest/config';
import vue from '@vitejs/plugin-vue';
import tailwindcss from '@tailwindcss/vite';
import { forwardedOrigin } from './dev/origin.ts';
import { tradeToken } from './dev/token.ts';

// The demo server (`npm run demo`, docs/api.md), on its default port. In development the UI runs here with instant
// reload and passes /api/ on to it.
const DEMO_PORT = 4787;
const SERVER = `http://127.0.0.1:${DEMO_PORT}`;

/** Opened with the token the demo printed (`?token=`), the page puts it into the demo server's cookie (dev/token.ts). */
function serverToken(): Plugin {
  return {
    name: 'warden-demo-token',
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const traded = req.method === 'GET' ? tradeToken(req.url ?? '/', DEMO_PORT) : undefined;
        if (!traded) {
          next();
          return;
        }
        res.statusCode = 303;
        res.setHeader('Location', traded.location);
        res.setHeader('Set-Cookie', traded.setCookie);
        res.end();
      });
    },
  };
}

export default defineConfig({
  plugins: [vue(), tailwindcss(), serverToken()],
  server: {
    host: '127.0.0.1',
    strictPort: false,
    proxy: {
      // The demo server accepts only its own Host and Origin: the UI's own requests are passed on as the server's,
      // anyone else's keep their Origin and are refused there (dev/origin.ts). The token's cookie goes with them.
      '/api': {
        target: SERVER,
        changeOrigin: true,
        configure: (proxy) =>
          proxy.on('proxyReq', (proxyReq, req) => {
            const origin = forwardedOrigin(req.headers.origin, req.headers.host, SERVER);
            if (origin !== undefined) proxyReq.setHeader('origin', origin);
          }),
      },
    },
  },
  // Every asset is a file of its own: the Content-Security-Policy allows images and fonts only from the app itself,
  // not inlined as data: URLs.
  build: { assetsInlineLimit: 0 },
  preview: { host: '127.0.0.1' },
  test: {
    environment: 'happy-dom',
    include: ['src/**/*.test.ts', 'dev/**/*.test.ts'],
  },
});
