import { defineConfig } from "@lovable.dev/vite-tanstack-config";

export default defineConfig({
  nitro: false,
  envDefine: false,
  tanstackStart: { spa: { enabled: true } },
  vite: {
    // This artifact has no build-time connection settings or credentials.
    envDir: false,
    envPrefix: [],
    preview: { host: "127.0.0.1" },
  },
});
