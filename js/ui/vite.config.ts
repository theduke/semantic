import { defineConfig } from "vite";
import solidPlugin from "vite-plugin-solid";

// const apiUrl = 'http://127.0.0.1:3000';
const apiUrl = 'http://192.168.0.66:8888';

export default defineConfig({
  plugins: [solidPlugin()],
  build: {
    target: "esnext",
  },
  server: {
    proxy: {
      "/api": apiUrl,
      "/blob": apiUrl,
    },
  },
});
