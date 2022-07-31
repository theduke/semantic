import { build, defineConfig } from 'vite';
import dts from 'vite-plugin-dts';

// libraries
const libraries = [
  {
    entry: './src/schema.ts',
    name: 'schema',
    fileName: 'schema',
  },
  {
    entry: './src/core.ts',
    name: 'core',
    fileName: 'core',
  },
  {
    entry: './src/db.ts',
    name: 'db',
    fileName: 'db',
  },
  {
    entry: './src/api.ts',
    name: 'api',
    fileName: 'api',
  },
];

// build
libraries.forEach(async (libItem) => {
  await build(defineConfig({
    configFile: false,
    plugins: [
      dts({insertTypesEntry: true}),
    ],
    build: {
      lib: libItem,
      emptyOutDir: false,
      // rollupOptions: {
      //   // other options
      // },
    },
  }));
});
