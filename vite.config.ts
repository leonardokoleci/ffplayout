import { fileURLToPath, URL } from 'node:url'

import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import vueDevTools from 'vite-plugin-vue-devtools'
import tailwindcss from '@tailwindcss/vite'

// https://vite.dev/config/
export default defineConfig({
    build: {
        chunkSizeWarningLimit: 1600,
    },
    root: './frontend',
    plugins: [
        tailwindcss(),
        vue({
            template: {
                compilerOptions: {
                    isCustomElement: (tag) => tag === 'hlsjs-video',
                },
            },
        }),
        vueDevTools(),
    ],
    resolve: {
        alias: {
            '@': fileURLToPath(new URL('./frontend/src', import.meta.url)),
        },
    },
    server: {
        host: process.env.FFPLAYOUT_DEV_HOST || '127.0.0.1',
        port: 5757,
        // Polling is needed when source is bind-mounted into a container,
        // because file-change events from Windows/Docker Desktop do not
        // propagate reliably.
        watch: {
            usePolling: process.env.FFPLAYOUT_DOCKER === '1',
        },
        proxy: {
            // /moq/player is a Vue route; proxy only bootstrap and media traffic.
            // Authenticated bootstrap (/api/moq/config) uses the /api proxy below.
            ...(Object.fromEntries(
                [
                    '/moq/config',
                    '/moq/relay',
                    '/api',
                    '/auth',
                    '/data',
                    '/live',
                    '/1/live',
                    '/public',
                    '/file',
                    '/hls',
                ].map((route) => [
                    route,
                    { target: process.env.FFPLAYOUT_PROXY_TARGET || 'http://127.0.0.1:8787', changeOrigin: true },
                ]),
            ) as Record<string, { target: string; changeOrigin: boolean }>),
            '/moq/relay': {
                target: process.env.FFPLAYOUT_PROXY_TARGET || 'http://127.0.0.1:8787',
                changeOrigin: true,
                ws: true,
            },
        },
    },
})
