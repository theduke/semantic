import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { build } from 'esbuild'

const directory = await mkdtemp(join(tmpdir(), 'dxeditor-bundle-'))
try {
  const output = join(directory, 'editor.iife.js')
  await build({
    entryPoints: ['src/index.ts'],
    bundle: true,
    format: 'iife',
    globalName: 'SemanticEditorEngine',
    target: 'es2020',
    minify: true,
    outfile: output,
  })
  const [generated, checkedIn] = await Promise.all([
    readFile(output),
    readFile('dist/editor.iife.js'),
  ])
  if (!generated.equals(checkedIn)) {
    throw new Error('dist/editor.iife.js is stale; run npm run build')
  }
} finally {
  await rm(directory, { recursive: true, force: true })
}
