import { beforeEach, describe, expect, it } from 'vitest'
import { Fragment, Schema, Slice } from '@tiptap/pm/model'
import urlPolicyCases from '../../assets/url_policy_cases.json'
import {
  decodeInternalClipboard,
  encodeInternalClipboard,
  INTERNAL_CLIPBOARD_VERSION,
  mount,
  parseTsvGrid,
  pmToV1,
  pmToV2,
  safeImageUrl,
  safeUrl,
  v1ToPm,
  v2ToPm,
  type ComponentDocumentV2,
  type EngineManifest,
} from './index'

beforeEach(() => {
  Range.prototype.getClientRects = () => ({
    0: new DOMRect(), length: 1, item: () => new DOMRect(),
    [Symbol.iterator]: function* () { yield new DOMRect() },
  }) as DOMRectList
  Range.prototype.getBoundingClientRect = () => new DOMRect()
})

const clipboardSchema = new Schema({
  nodes: {
    doc: { content: 'block+' },
    paragraph: { group: 'block', content: 'inline*', attrs: { semanticId: { default: null } } },
    text: { group: 'inline' },
    image: { group: 'inline', inline: true, attrs: { semanticId: { default: null }, src: {}, alt: { default: null }, title: { default: null } } },
  },
  marks: { link: { attrs: { href: {} } } },
})

describe('document wire adapters', () => {
  it('round trips the native component document v2 protocol', () => {
    const document: ComponentDocumentV2 = {
      schema: 'semantic.component-document', version: 2, metadata: { source: 'fixture' },
      root: { kind: 'document', content: [{
        kind: 'table', id: 'table-1', content: [{ kind: 'table_row', id: 'row-1', content: [{
          kind: 'table_header', id: 'cell-1', attrs: { colspan: 1, rowspan: 1, alignment: 'center' },
          content: [{ kind: 'paragraph', id: 'paragraph-1', content: [{
            kind: 'text', text: 'Title', marks: [{ kind: 'bold' }, { kind: 'link', attrs: { href: 'https://example.com', title: 'Example' } }],
          }] }],
        }] }],
      }] },
    }
    const roundTrip = pmToV2(v2ToPm(document), document.metadata)
    expect(roundTrip).toMatchObject({
      schema: 'semantic.component-document', version: 2, metadata: { source: 'fixture' },
      root: { content: [{ kind: 'table', id: 'table-1', content: [{ content: [{
        kind: 'table_header', attrs: { alignment: 'center' }, content: [{ content: [{
          kind: 'text', text: 'Title', marks: [{ kind: 'bold' }, { kind: 'link', attrs: { href: 'https://example.com', title: 'Example' } }],
        }] }],
      }] }] }] },
    })
  })

  it('preserves unknown v2 nodes as inert opaque nodes', () => {
    const document: ComponentDocumentV2 = {
      schema: 'semantic.component-document', version: 2,
      root: { kind: 'document', content: [{ kind: 'future_widget', id: 'future-1', attrs: { fallback: 'Widget' } }] },
    }
    expect(pmToV2(v2ToPm(document)).root.content?.[0]).toEqual(document.root.content?.[0])
  })

  it('round trips structured blocks and safe marks', () => {
    const document = {
      schema: 'dxeditor.document.v1',
      meta: {},
      blocks: [{
        id: 'heading-1',
        component: 'heading',
        attrs: { level: 2 },
        content: { kind: 'Inline' as const, value: [{
          id: 'text-1',
          component: 'text',
          attrs: {},
          text: 'A title',
          marks: [{ component: 'bold', attrs: {} }],
        }] },
      }],
    }

    const roundTrip = pmToV1(v1ToPm(document))
    expect(roundTrip.blocks[0]).toMatchObject({
      id: 'heading-1',
      component: 'heading',
      attrs: { level: 2 },
      content: { kind: 'Inline', value: [{ text: 'A title', marks: [{ component: 'bold' }] }] },
    })
  })

  it('drops unsafe link marks at the engine boundary', () => {
    const pm = v1ToPm({
      schema: 'dxeditor.document.v1',
      meta: {},
      blocks: [{
        id: 'block-1', component: 'paragraph', attrs: {},
        content: { kind: 'Inline', value: [{
          id: 'text-1', component: 'text', attrs: {}, text: 'bad',
          marks: [{ component: 'link', attrs: { href: 'javascript:alert(1)' } }],
        }] },
      }],
    })
    expect(pm.content?.[0].content?.[0].marks).toEqual([])
  })

  it('preserves nested quotes, tasks, and inert raw HTML', () => {
    const document = {
      schema: 'dxeditor.document.v1', meta: {}, blocks: [
        { id: 'quote-1', component: 'quote', attrs: {}, content: { kind: 'Blocks' as const, value: [
          { id: 'paragraph-1', component: 'paragraph', attrs: {}, content: { kind: 'Inline' as const, value: [
            { id: 'raw-inline', component: 'raw_html', attrs: {}, text: '<b>literal</b>', marks: [] },
          ] } },
        ] } },
        { id: 'list-1', component: 'list', attrs: { ordered: false }, content: { kind: 'Blocks' as const, value: [
          { id: 'item-1', component: 'list_item', attrs: { checked: true }, content: { kind: 'Blocks' as const, value: [
            { id: 'paragraph-2', component: 'paragraph', attrs: {}, content: { kind: 'Inline' as const, value: [
              { id: 'text-2', component: 'text', attrs: {}, text: 'done', marks: [] },
            ] } },
          ] } },
        ] } },
      ],
    }
    const pm = v1ToPm(document)
    expect(pm.content?.[0].type).toBe('blockquote')
    expect(pm.content?.[1].type).toBe('taskList')
    expect(pm.content?.[0].content?.[0].content?.[0]).toMatchObject({ type: 'opaqueInline', attrs: { source: '<b>literal</b>' } })
    expect(pmToV1(pm).blocks[0].content.kind).toBe('Blocks')
  })

  it('round trips inline images with accessible metadata', () => {
    const document = {
      schema: 'dxeditor.document.v1', meta: {}, blocks: [{
        id: 'paragraph-1', component: 'paragraph', attrs: {}, content: { kind: 'Inline' as const, value: [{
          id: 'image-1', component: 'image', attrs: { src: 'https://example.com/image.png', title: 'Diagram' }, text: 'Architecture diagram', marks: [],
        }] },
      }],
    }
    expect(pmToV1(v1ToPm(document)).blocks[0]).toMatchObject({
      content: { value: [{ component: 'image', text: 'Architecture diagram', attrs: { src: 'https://example.com/image.png', title: 'Diagram' } }] },
    })
  })
})

describe('internal clipboard envelope', () => {
  it('accepts matching versioned schema content', () => {
    const paragraph = clipboardSchema.node('paragraph', null, clipboardSchema.text('internal'))
    const encoded = encodeInternalClipboard(new Slice(paragraph.content, 0, 0))
    expect(decodeInternalClipboard(encoded, clipboardSchema)?.content.textBetween(0, 8)).toBe('internal')
  })

  it('rejects mismatched versions and unsafe URLs', () => {
    const paragraph = clipboardSchema.node('paragraph', null, clipboardSchema.text('safe'))
    const encoded = JSON.parse(encodeInternalClipboard(new Slice(paragraph.content, 0, 0)))
    encoded.version = INTERNAL_CLIPBOARD_VERSION + 1
    expect(decodeInternalClipboard(JSON.stringify(encoded), clipboardSchema)).toBeNull()

    encoded.version = INTERNAL_CLIPBOARD_VERSION
    encoded.slice = { content: [{ type: 'image', attrs: { src: 'javascript:alert(1)' } }] }
    expect(decodeInternalClipboard(JSON.stringify(encoded), clipboardSchema)).toBeNull()
  })

  it('binds clipboard payloads to the active Rust catalog fingerprint', () => {
    const paragraph = clipboardSchema.node('paragraph', null, clipboardSchema.text('safe'))
    const encoded = encodeInternalClipboard(new Slice(paragraph.content, 0, 0), 'catalog-a')
    expect(decodeInternalClipboard(encoded, clipboardSchema, 'catalog-a')).not.toBeNull()
    expect(decodeInternalClipboard(encoded, clipboardSchema, 'catalog-b')).toBeNull()
  })

  it('strips copied semantic identities before the slice is inserted', () => {
    const paragraph = clipboardSchema.node('paragraph', { semanticId: 'paragraph-source' }, [
      clipboardSchema.node('image', { semanticId: 'image-source', src: 'https://example.com/a.png' }),
    ])
    const decoded = decodeInternalClipboard(
      encodeInternalClipboard(new Slice(Fragment.from(paragraph), 0, 0)), clipboardSchema,
    )
    expect(decoded?.content.firstChild?.attrs.semanticId).toBeNull()
    expect(decoded?.content.firstChild?.firstChild?.attrs.semanticId).toBeNull()
  })
})

describe('rectangular TSV parsing', () => {
  it('preserves quoted tabs and grows short rows into a rectangle', () => {
    expect(parseTsvGrid('name\tvalue\n"one\ttwo"\t3\nlast')).toEqual([
      ['name', 'value'], ['one\ttwo', '3'], ['last', ''],
    ])
  })

  it('leaves ordinary text and malformed quoted data to normal paste', () => {
    expect(parseTsvGrid('ordinary text')).toBeNull()
    expect(parseTsvGrid('"unterminated\tvalue')).toBeNull()
  })
})

describe('URL role policies', () => {
  it('matches the shared hyperlink and media conformance corpus', () => {
    for (const fixture of urlPolicyCases) {
      expect(safeUrl(fixture.value), `hyperlink: ${fixture.value}`).toBe(fixture.hyperlink)
      expect(safeImageUrl(fixture.value), `media: ${fixture.value}`).toBe(fixture.media)
    }
  })
})

const paragraphDocument = (id: string, text: string): ComponentDocumentV2 => ({
  schema: 'semantic.component-document',
  version: 2,
  root: { kind: 'document', content: [{
    kind: 'paragraph', id, content: text ? [{ kind: 'text', text }] : [],
  }] },
})

const allIds = (document: ComponentDocumentV2): string[] => {
  const ids: string[] = []
  const visit = (node: NonNullable<ComponentDocumentV2['root']>) => {
    if (node.id) ids.push(node.id)
    node.content?.forEach(visit)
  }
  visit(document.root)
  return ids
}

describe('session durability, identity, and history', () => {
  it('emits a document-changing command synchronously and destroy does not duplicate it', () => {
    document.body.innerHTML = '<div class="dxeditor"><div id="host"></div></div>'
    const events: Array<{ kind?: string; revision?: number }> = []
    const session = mount(document.querySelector('#host')!, {
      sessionId: 'session-sync', document: paragraphDocument('paragraph-1', 'one'), readonly: false,
      emit: event => events.push(event as { kind?: string; revision?: number }),
    })
    expect(session.command('horizontalRule')).toBe(true)
    const changes = events.filter(event => event.kind === 'documentChange')
    expect(changes).toHaveLength(1)
    expect(changes[0]?.revision).toBe(1)
    session.destroy()
    expect(events.filter(event => event.kind === 'documentChange')).toHaveLength(1)
  })

  it('keeps generated IDs stable and unique across repeated snapshots and table growth', () => {
    document.body.innerHTML = '<div class="dxeditor"><div id="host"></div></div>'
    const session = mount(document.querySelector('#host')!, {
      sessionId: 'session-ids', document: paragraphDocument('paragraph-1', 'one'), readonly: false,
      emit: () => {},
    })
    expect(session.command('table')).toBe(true)
    expect(session.command('addRowAfter')).toBe(true)
    const first = session.snapshot()
    const second = session.snapshot()
    expect(second).toEqual(first)
    const ids = allIds(first)
    expect(ids.every(Boolean)).toBe(true)
    expect(new Set(ids).size).toBe(ids.length)
    session.destroy()
  })

  it('reset replacement clears history so undo cannot resurrect the old revision', () => {
    document.body.innerHTML = '<div class="dxeditor"><div id="host"></div></div>'
    const session = mount(document.querySelector('#host')!, {
      sessionId: 'session-history', document: paragraphDocument('paragraph-1', 'local'), readonly: false,
      emit: () => {},
    })
    expect(session.command('horizontalRule')).toBe(true)
    session.replaceDocument(paragraphDocument('paragraph-server', 'server'), 'reset')
    session.command('undo')
    expect(session.snapshot().root.content).toEqual(paragraphDocument('paragraph-server', 'server').root.content)
    session.destroy()
  })

  it('preserve replacement maps retained history without restoring replaced text', () => {
    document.body.innerHTML = '<div class="dxeditor"><div id="host"></div></div>'
    const session = mount(document.querySelector('#host')!, {
      sessionId: 'session-preserve', document: paragraphDocument('paragraph-1', 'local'), readonly: false,
      emit: () => {},
    })
    expect(session.command('heading', { level: 1 })).toBe(true)
    session.replaceDocument(paragraphDocument('paragraph-server', 'server'), 'preserve')
    session.command('undo')
    expect(session.snapshot().root.content?.[0]?.content?.[0]?.text).toBe('server')
    session.destroy()
  })

  it('rejects missing and duplicate imported identities before mount', () => {
    const missing = paragraphDocument('', 'missing')
    missing.root.content![0]!.id = undefined
    expect(() => v2ToPm(missing)).toThrow(/missing semanticId/)
    const duplicate = paragraphDocument('duplicate', 'one')
    duplicate.root.content!.push({ kind: 'paragraph', id: 'duplicate', content: [] })
    expect(() => v2ToPm(duplicate)).toThrow(/duplicate semanticId/)
  })
})

const engineManifest = (overrides: Partial<EngineManifest> = {}): EngineManifest => ({
  version: 1,
  document_catalog_fingerprint: 'fixture-catalog',
  clipboard_schema_fingerprint: 'fixture-clipboard',
  format_id: 'markdown',
  aria_label: 'Fixture editor',
  components: [],
  commands: [],
  features: {
    table_headers: true, table_alignment: true, persistent_table_widths: false,
    table_spans: false, media: true, tasks: true, opaque_content: true,
  },
  ...overrides,
})

describe('engine capability manifest', () => {
  it('fails mount with an actionable missing adapter error', () => {
    document.body.innerHTML = '<div class="dxeditor"><div id="host"></div></div>'
    expect(() => mount(document.querySelector('#host')!, {
      sessionId: 'session-manifest', schemaFingerprint: 'fixture-catalog',
      document: paragraphDocument('paragraph-1', 'one'), readonly: false, emit: () => {},
      manifest: engineManifest({
        components: [{ id: 'widget', adapter: 'missing', format_capability: 'native' }],
      }),
    })).toThrow(/component 'widget'.*missing behavior adapter 'missing'/)
  })

  it('does not expose commands disabled by the active output format', () => {
    document.body.innerHTML = '<div class="dxeditor"><div id="host"></div></div>'
    const session = mount(document.querySelector('#host')!, {
      sessionId: 'session-capability', schemaFingerprint: 'fixture-catalog',
      document: paragraphDocument('paragraph-1', 'one'), readonly: false, emit: () => {},
      manifest: engineManifest({
        commands: [{ id: 'image', enabled: false, disabled_reason: 'not persistable' }],
      }),
    })
    expect(session.command('image', { src: 'https://example.com/a.png' })).toBe(false)
    session.destroy()
  })
})
