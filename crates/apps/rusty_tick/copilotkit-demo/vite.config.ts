import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// The page is served by Vite; /api/copilotkit and /api/demo go to runtime.mjs.
const runtime = "http://127.0.0.1:4000";
export default defineConfig({
  plugins: [react()],
  server: { proxy: { "/api/copilotkit": runtime, "/api/demo": runtime } },
});
