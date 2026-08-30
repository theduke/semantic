import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { Fragment, Schema, Slice } from '@tiptap/pm/model'
import urlPolicyCases from '../../assets/url_policy_cases.json'
import {
  decodeInternalClipboard,
  entityHref,
  entityIdFromHref,
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
import { plainTextParagraphSlice } from './core/paragraph-behavior'

beforeEach(() => {
  Range.prototype.getClientRects = () => ({
    0: new DOMRect(), length: 1, item: () => new DOMRect(),
    [Symbol.iterator]: function* () { yield new DOMRect() },
  }) as DOMRectList
  Range.prototype.getBoundingClientRect = () => new DOMRect()
})

afterEach(() => vi.useRealTimers())

const clipboardSchema = new Schema({
  nodes: {
    doc: { content: 'block+' },
    paragraph: { group: 'block', content: 'inline*', attrs: { semanticId: { default: null } } },
    text: { group: 'inline' },
    hardBreak: { group: 'inline', inline: true, attrs: { semanticId: { default: null } } },
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

  it('keeps a multiline paragraph and its semantic identities intact', () => {
    const document: ComponentDocumentV2 = {
      schema: 'semantic.component-document', version: 2,
      root: { kind: 'document', content: [{
        kind: 'paragraph', id: 'paragraph-multiline', content: [
          { kind: 'text', text: 'first' },
          { kind: 'hard_break', id: 'break-1' },
          { kind: 'text', text: 'second' },
        ],
      }] },
    }
    expect(pmToV2(v2ToPm(document))).toEqual(document)
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

  it('preserves legacy inline hard-break identity', () => {
    const document = {
      schema: 'dxeditor.document.v1', meta: {}, blocks: [{
        id: 'paragraph-1', component: 'paragraph', attrs: {},
        content: { kind: 'Inline' as const, value: [
          { id: 'text-1', component: 'text', attrs: {}, text: 'first', marks: [] },
          { id: 'break-1', component: 'hard_break', attrs: {}, text: '\n', marks: [] },
          { id: 'text-2', component: 'text', attrs: {}, text: 'second', marks: [] },
        ] },
      }],
    }
    const roundTrip = pmToV1(v1ToPm(document))
    expect(roundTrip.blocks[0]?.id).toBe('paragraph-1')
    const inline = roundTrip.blocks[0]?.content.value as Array<{ id: string; component: string; text: string }>
    expect(inline[1]).toMatchObject({
      id: 'break-1', component: 'hard_break', text: '\n',
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

describe('plain text paragraph parsing', () => {
  it('keeps single newlines inline and uses blank lines as block boundaries', () => {
    const slice = plainTextParagraphSlice('alpha\r\nbeta\n\ngamma', clipboardSchema)
    expect(slice.content.toJSON()).toEqual([
      { type: 'paragraph', attrs: { semanticId: null }, content: [
        { type: 'text', text: 'alpha' },
        { type: 'hardBreak', attrs: { semanticId: null } },
        { type: 'text', text: 'beta' },
      ] },
      { type: 'paragraph', attrs: { semanticId: null }, content: [{ type: 'text', text: 'gamma' }] },
    ])
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

describe('internal entity link extension', () => {
  const entityDocument = (): ComponentDocumentV2 => ({
    schema: 'semantic.component-document', version: 2,
    root: { kind: 'document', content: [{
      kind: 'paragraph', id: 'paragraph-entity', content: [
        { kind: 'mention', id: 'mention-1', attrs: { entity_id: 'person:1', label: 'Ada' } },
      ],
    }] },
  })

  it('preserves the semantic entity URI contract and v2 round trip', () => {
    expect(entityHref('person:1')).toBe('semantic:entity:person:1')
    expect(entityIdFromHref('semantic:entity:person:1')).toBe('person:1')
    expect(entityIdFromHref('https://example.com')).toBeNull()
    expect(pmToV2(v2ToPm(entityDocument()))).toEqual(entityDocument())
  })

  it('registers /entity only when an application search provider exists', () => {
    document.body.innerHTML = '<div class="dxeditor"><div id="host"></div><div data-dxeditor-overlays></div></div>'
    const withoutProvider = mount(document.querySelector('#host')!, {
      sessionId: 'entity-none', document: paragraphDocument('paragraph-none', ''), readonly: false, emit: () => {},
    })
    ;(document.querySelector('[title="Add a block"]') as HTMLButtonElement).click()
    expect(document.querySelector('[data-extension="entity-link"]')).toBeNull()
    withoutProvider.destroy()

    document.body.innerHTML = '<div class="dxeditor"><div id="host"></div><div data-dxeditor-overlays></div></div>'
    const withProvider = mount(document.querySelector('#host')!, {
      sessionId: 'entity-enabled', document: paragraphDocument('paragraph-enabled', ''), readonly: false, emit: () => {},
      entitySearchProvider: () => [],
    })
    ;(document.querySelector('[title="Add a block"]') as HTMLButtonElement).click()
    expect(document.querySelector('[data-extension="entity-link"]')).not.toBeNull()
    withProvider.destroy()
  })

  it('discards stale async search and inserts the selected entity link', async () => {
    vi.useFakeTimers()
    document.body.innerHTML = '<div class="dxeditor"><div id="host"></div><div data-dxeditor-overlays></div></div>'
    const provider = vi.fn((query: string, context: { signal: AbortSignal }) => new Promise<readonly { id: string; label: string }[]>(resolve => {
      const delay = query === 'old' ? 80 : 10
      setTimeout(() => {
        if (!context.signal.aborted) resolve([{ id: query || 'all', label: query || 'All' }])
      }, delay)
    }))
    const session = mount(document.querySelector('#host')!, {
      sessionId: 'entity-search', document: paragraphDocument('paragraph-search', ''), readonly: false, emit: () => {},
      entitySearchProvider: provider,
    })
    ;(document.querySelector('[title="Add a block"]') as HTMLButtonElement).click()
    ;(document.querySelector('[data-extension="entity-link"]') as HTMLButtonElement).click()
    const search = document.querySelector('[aria-label="Search entities"]') as HTMLInputElement
    search.value = 'old'; search.dispatchEvent(new Event('input'))
    await vi.advanceTimersByTimeAsync(120)
    search.value = 'new'; search.dispatchEvent(new Event('input'))
    await vi.advanceTimersByTimeAsync(140)
    expect(document.querySelector('[title="Link to new"]')).not.toBeNull()
    expect(document.querySelector('[title="Link to old"]')).toBeNull()
    ;(document.querySelector('[title="Link to new"]') as HTMLButtonElement).click()
    expect(session.snapshot().root.content?.[0]?.content?.[0]).toMatchObject({
      kind: 'mention', attrs: { entity_id: 'new', label: 'new' },
    })
    session.destroy()
  })

  it('renders provider-owned hover preview data without interpreting HTML', async () => {
    document.body.innerHTML = '<div class="dxeditor"><div id="host"></div><div data-dxeditor-overlays></div></div>'
    const preview = vi.fn(async () => ({
      id: 'person:1', label: '<b>Ada</b>', detail: 'Person', fields: [{ label: 'Email', value: 'ada@example.test' }],
    }))
    const open = vi.fn()
    const session = mount(document.querySelector('#host')!, {
      sessionId: 'entity-preview', document: entityDocument(), readonly: false, emit: () => {},
      entitySearchProvider: () => [], entityPreviewProvider: preview, entityOpenHandler: open, entityLinkColor: 'rebeccapurple',
    })
    const link = document.querySelector('[data-semantic-mention="person:1"]') as HTMLElement
    expect(link.classList.contains('dxeditor-engine__entity-link')).toBe(true)
    link.dispatchEvent(new Event('pointerover', { bubbles: true }))
    await vi.waitFor(() => expect(document.querySelector('[aria-label="Entity preview"]')?.textContent).toContain('<b>Ada</b>'))
    expect(document.querySelector('[aria-label="Entity preview"] b')).toBeNull()
    expect(document.querySelector('[aria-label="Entity preview"]')?.textContent).toContain('ada@example.test')
    link.click()
    expect(open).toHaveBeenCalledWith('person:1')
    session.destroy()
  })
})

const paragraphDocument = (id: string, text: string): ComponentDocumentV2 => ({
  schema: 'semantic.component-document',
  version: 2,
  root: { kind: 'document', content: [{
    kind: 'paragraph', id, content: text ? [{ kind: 'text', text }] : [],
  }] },
})

const tableDocument = (rows = 3, columns = 2): ComponentDocumentV2 => ({
  schema: 'semantic.component-document',
  version: 2,
  root: { kind: 'document', content: [{
    kind: 'table', id: 'table-1', content: Array.from({ length: rows }, (_, row) => ({
      kind: 'table_row', id: `row-${row + 1}`, content: Array.from({ length: columns }, (_, column) => ({
        kind: row === 0 ? 'table_header' : 'table_cell',
        id: `cell-${row + 1}-${column + 1}`,
        content: [{
          kind: 'paragraph', id: `paragraph-${row + 1}-${column + 1}`,
          content: [{ kind: 'text', text: `R${row + 1}C${column + 1}` }],
        }],
      })),
    })),
  }] },
})

describe('table widget controls', () => {
  it('targets explicit rows independently of the cell selection and honors boundaries', () => {
    document.body.innerHTML = '<div class="dxeditor"><div id="host"></div><div data-dxeditor-overlays></div></div>'
    const session = mount(document.querySelector('#host')!, {
      sessionId: 'table-rows', document: tableDocument(), readonly: false, emit: () => {},
    })

    const handles = () => Array.from(document.querySelectorAll<HTMLButtonElement>('.dxeditor-engine__table-row-handle'))
    expect(handles().map(handle => handle.getAttribute('aria-label'))).toEqual([
      'Row 1 actions', 'Row 2 actions', 'Row 3 actions',
    ])
    handles()[0]!.click()
    expect((document.querySelector('[title="Move row up"]') as HTMLButtonElement).disabled).toBe(true)
    ;(document.querySelector('[title="Move row up"]') as HTMLButtonElement).click()
    expect(session.snapshot().root.content?.[0]?.content?.map(row => row.id)).toEqual(['row-1', 'row-2', 'row-3'])

    handles()[1]!.click()
    expect((document.querySelector('[title="Move row down"]') as HTMLButtonElement).disabled).toBe(false)
    ;(document.querySelector('[title="Move row down"]') as HTMLButtonElement).click()
    expect(session.snapshot().root.content?.[0]?.content?.map(row => row.id)).toEqual(['row-1', 'row-3', 'row-2'])

    const movedHandle = handles().find(handle => handle.dataset.rowId === 'row-2')!
    movedHandle.click()
    expect((document.querySelector('[title="Move row down"]') as HTMLButtonElement).disabled).toBe(true)
    ;(document.querySelector('[title="Delete row"]') as HTMLButtonElement).click()
    expect(session.snapshot().root.content?.[0]?.content?.map(row => row.id)).toEqual(['row-1', 'row-3'])
    session.destroy()
  })

  it('adds only at the table edges and does not expose editing controls in readonly mode', () => {
    document.body.innerHTML = '<div class="dxeditor"><div id="host"></div><div data-dxeditor-overlays></div></div>'
    const session = mount(document.querySelector('#host')!, {
      sessionId: 'table-edges', document: tableDocument(2, 2), readonly: false, emit: () => {},
    })
    ;(document.querySelector('[title="Add row at bottom"]') as HTMLButtonElement).click()
    ;(document.querySelector('[title="Add column at right"]') as HTMLButtonElement).click()
    const table = session.snapshot().root.content?.[0]
    expect(table?.content).toHaveLength(3)
    expect(table?.content?.every(row => row.content?.length === 3)).toBe(true)
    session.destroy()

    document.body.innerHTML = '<div class="dxeditor"><div id="host"></div><div data-dxeditor-overlays></div></div>'
    const readonly = mount(document.querySelector('#host')!, {
      sessionId: 'table-readonly', document: tableDocument(), readonly: true, emit: () => {},
    })
    document.querySelector('tr')?.dispatchEvent(new Event('pointermove', { bubbles: true }))
    expect((document.querySelector('[aria-label="Table actions"]') as HTMLElement).hidden).toBe(true)
    expect((document.querySelector('[aria-label="Table row actions"]') as HTMLElement).hidden).toBe(true)
    expect(document.querySelectorAll('.dxeditor-engine__table-row-handle')).toHaveLength(0)
    readonly.destroy()
  })
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
