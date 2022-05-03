import { defineConfig } from "vite";
import solidPlugin from "vite-plugin-solid";

export default defineConfig({
  plugins: [solidPlugin()],
  build: {
    target: "esnext",
    polyfillDynamicImport: false,
  },
  server: {
    proxy: {
      "/api": "http://localhost:3000",
      "/blob": "http://localhost:3000",
    },
  },
});
