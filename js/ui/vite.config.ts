import { defineConfig } from "vite";
import solidPlugin from "vite-plugin-solid";

export default defineConfig({
  plugins: [solidPlugin()],
  build: {
    target: "esnext",
  },
  server: {
    proxy: {
      "/api": "http://127.0.0.1:3000",
      "/blob": "http://127.0.0.1:3000",
    },
  },
});
