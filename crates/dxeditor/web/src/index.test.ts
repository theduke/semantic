import { describe, expect, it } from 'vitest'
import { Schema, Slice } from '@tiptap/pm/model'
import {
  decodeInternalClipboard,
  encodeInternalClipboard,
  INTERNAL_CLIPBOARD_VERSION,
  parseTsvGrid,
  pmToV1,
  pmToV2,
  v1ToPm,
  v2ToPm,
  type ComponentDocumentV2,
} from './index'

const clipboardSchema = new Schema({
  nodes: {
    doc: { content: 'block+' },
    paragraph: { group: 'block', content: 'inline*' },
    text: { group: 'inline' },
    image: { group: 'inline', inline: true, attrs: { src: {}, alt: { default: null }, title: { default: null } } },
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
