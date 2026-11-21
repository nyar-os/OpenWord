import { defineConfig } from '@vmz/vmz';

/**
 * Desktop studio shell served by the WAE WebView host.
 * Profile `static` → assembly `web-static`; the native host loads `dist`.
 */
export default defineConfig({
    delivery: {
        default: 'static',
        profiles: {
            static: {
                host: 'browser',
                assembly: 'web-static',
            },
        },
    },
});
