import {defineConfig} from 'vite';
import path from 'path';
import dts from 'vite-plugin-dts';

export default defineConfig({
  plugins: [
    dts({insertTypesEntry: true}),
  ],
  build: {
    // lib: {
    //   entry: path.resolve(__dirname, 'src/index.ts'),
    //   name: 'semantic',
    //   formats: ['es', 'umd'],
    // },
    rollupOptions: {
      input: {
        index: path.resolve(__dirname, 'src/index.ts'),
        core: path.resolve(__dirname, 'src/core.ts'),
        schema: path.resolve(__dirname, 'src/schema.ts'),
        api: path.resolve(__dirname, 'src/api.ts'),
        db: path.resolve(__dirname, 'src/db.ts'),
      }
    }
  },
});
