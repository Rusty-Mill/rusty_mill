import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import tailwindcss from "@tailwindcss/vite";

// Tauri drives the dev server; keep its port fixed (matches devUrl in
// tauri.conf.json) and don't let Vite clobber Tauri's console output.
export default defineConfig({
  plugins: [solid(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    target: "es2021",
    outDir: "dist",
    emptyOutDir: true,
  },
});
