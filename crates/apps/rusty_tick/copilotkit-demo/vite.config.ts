import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// The page is served by Vite; /api/copilotkit goes to runtime.mjs.
export default defineConfig({
  plugins: [react()],
  server: { proxy: { "/api/copilotkit": "http://127.0.0.1:4000" } },
});
