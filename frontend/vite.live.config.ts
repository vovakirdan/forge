import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// The live entry does not import TanStack Start, demo routes, or Lovable plugins.
export default defineConfig({
  root: fileURLToPath(new URL("./src/live", import.meta.url)),
  publicDir: false,
  envDir: false,
  envPrefix: [],
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) } },
  build: {
    outDir: fileURLToPath(new URL("./dist-live", import.meta.url)),
    emptyOutDir: true,
    sourcemap: false,
  },
});
