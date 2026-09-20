import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [react()],
  build: {
    rollupOptions: {
      output: {
        // The function form, not the `{ name: [packages] }` object: Vite 8
        // bundles with Rolldown, whose `manualChunks` accepts only a
        // function (`tsc -b` rejects the object under its types), and the
        // function form is equally valid on Vite 5's Rollup. Same three
        // vendor chunks as before, keyed by the package's node_modules path.
        manualChunks(id: string) {
          const path = id.replace(/\\/g, '/');
          if (!path.includes('/node_modules/')) return undefined;
          if (/\/node_modules\/(react|react-dom|scheduler)\//.test(path)) return 'react';
          if (path.includes('/node_modules/@xterm/')) return 'xterm';
          if (path.includes('/node_modules/@anthropic-ai/')) return 'anthropic';
          return undefined;
        },
      },
    },
  },
});
