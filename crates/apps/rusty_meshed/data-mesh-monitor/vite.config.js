import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Where the registry API lives. Defaults to the `meshed-registry` binary's
// own default bind (`cargo run -p rusty-meshed-registry --bin meshed_registry`,
// 127.0.0.1:8100 -- the same port the source repo's `uvicorn ... --port 8100`
// used); override with MESHED_REGISTRY_URL for a registry running elsewhere.
const registry = process.env.MESHED_REGISTRY_URL ?? "http://localhost:8100";

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      "/api/mesh": {
        target: registry,
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/api\/mesh/, "/monitor"),
      },
      "/api/transform": {
        target: registry,
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/api\/transform/, "/transformation"),
      },
    },
  },
});
