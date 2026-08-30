import { Editor, Node, mergeAttributes, type JSONContent } from '@tiptap/core'
import Document from '@tiptap/extension-document'
import Paragraph from '@tiptap/extension-paragraph'
import Text from '@tiptap/extension-text'
import Heading from '@tiptap/extension-heading'
import Bold from '@tiptap/extension-bold'
import Italic from '@tiptap/extension-italic'
import Strike from '@tiptap/extension-strike'
import Code from '@tiptap/extension-code'
import Blockquote from '@tiptap/extension-blockquote'
import CodeBlock from '@tiptap/extension-code-block'
import BulletList from '@tiptap/extension-bullet-list'
import OrderedList from '@tiptap/extension-ordered-list'
import ListItem from '@tiptap/extension-list-item'
import HardBreak from '@tiptap/extension-hard-break'
import HorizontalRule from '@tiptap/extension-horizontal-rule'
import Link from '@tiptap/extension-link'
import Image from '@tiptap/extension-image'
import { Table } from '@tiptap/extension-table'
import TableRow from '@tiptap/extension-table-row'
import TableCell from '@tiptap/extension-table-cell'
import TableHeader from '@tiptap/extension-table-header'
import TaskList from '@tiptap/extension-task-list'
import TaskItem from '@tiptap/extension-task-item'
import { Dropcursor, Gapcursor, UndoRedo } from '@tiptap/extensions'
import { Slice, type Schema } from '@tiptap/pm/model'
import { EditorState } from '@tiptap/pm/state'
import { CellSelection } from '@tiptap/pm/tables'
import { CoreParagraphBehavior, plainTextParagraphSlice } from './core/paragraph-behavior'
import { SemanticId } from './extensions/semantic-id'
import { stripCopiedIdentities, validateSemanticIds } from './identity'

type Attributes = Record<string, unknown>
type MarkV1 = { component: string; attrs: Attributes }
type InlineV1 = { id: string; component: string; attrs: Attributes; text: string; marks: MarkV1[] }
type ContentV1 = { kind: 'Inline' | 'Blocks' | 'Table' | 'Void' | 'Custom'; value?: unknown }
type BlockV1 = { id: string; component: string; attrs: Attributes; content: ContentV1 }
type DocumentV1 = { schema: string; blocks: BlockV1[]; meta: Attributes }
type CellV1 = { id: string; attrs: Attributes; blocks: BlockV1[] }
type RowV1 = { id: string; attrs: Attributes; cells: CellV1[] }
type TableV1 = { rows: RowV1[] }

type ComponentMarkV2 = { kind: string; attrs?: Attributes }
type ComponentNodeV2 = {
  kind: string
  id?: string
  attrs?: Attributes
  content?: ComponentNodeV2[]
  text?: string
  marks?: ComponentMarkV2[]
}
export type ComponentDocumentV2 = {
  schema: 'semantic.component-document'
  version: 2
  root: ComponentNodeV2
  metadata?: Attributes
}

export type MentionCandidate = { id: string; label: string; detail?: string }
export type EngineManifest = {
  version: number
  document_catalog_fingerprint: string
  clipboard_schema_fingerprint: string
  format_id: string
  aria_label: string
  components: Array<{ id: string; adapter: string; format_capability: 'native' | 'opaque' | 'unsupported' }>
  commands: Array<{ id: string; enabled: boolean; disabled_reason?: string }>
  features: {
    table_headers: boolean
    table_alignment: boolean
    persistent_table_widths: boolean
    table_spans: boolean
    media: boolean
    tasks: boolean
    opaque_content: boolean
  }
}
export type MentionProvider = (
  query: string,
  context: { signal: AbortSignal; sessionId: string },
) => Promise<readonly MentionCandidate[]> | readonly MentionCandidate[]

export interface MountOptions {
  sessionId: string
  schemaFingerprint?: string
  document: ComponentDocumentV2
  readonly: boolean
  ariaLabel?: string
  formatId?: string
  manifest?: EngineManifest
  emit: (event: unknown) => void
  mentionProvider?: MentionProvider
}

export interface EditorSession {
  command(command: string, attrs?: Attributes): boolean
  replaceDocument(document: ComponentDocumentV2, historyPolicy?: 'preserve' | 'reset'): void
  snapshot(): ComponentDocumentV2
  destroy(): void
}

export const safeUrl = (value: string): boolean => {
  if (!value || value !== value.trim() || /[\s\u0000-\u001f\u007f\\]/u.test(value)) return false
  try {
    const parsed = new URL(value, window.location.origin)
    return ['http:', 'https:', 'mailto:', 'tel:'].includes(parsed.protocol)
  } catch {
    return false
  }
}

export const safeImageUrl = (value: string): boolean => {
  if (!value || value !== value.trim() || /[\s\u0000-\u001f\u007f\\]/u.test(value) || !/^https?:\/\//iu.test(value)) return false
  try {
    return ['http:', 'https:'].includes(new URL(value).protocol)
  } catch {
    return false
  }
}

export const INTERNAL_CLIPBOARD_MIME = 'application/x-semantic-dxeditor+json'
export const INTERNAL_CLIPBOARD_VERSION = 2
export const INTERNAL_SCHEMA_FINGERPRINT = 'semantic.pm-slice.v2:2026-08-30'

type ClipboardEnvelope = {
  version: number
  schema: string
  schemaFingerprint: string
  slice: { content?: JSONContent[]; openStart?: number; openEnd?: number }
}

const allowedClipboardNodes = new Set([
  'paragraph', 'heading', 'text', 'blockquote', 'codeBlock', 'bulletList', 'orderedList',
  'listItem', 'taskList', 'taskItem', 'hardBreak', 'horizontalRule', 'image', 'table',
  'tableRow', 'tableCell', 'tableHeader', 'semanticMention', 'opaqueBlock', 'opaqueInline',
])
const allowedClipboardMarks = new Set(['bold', 'italic', 'strike', 'code', 'link'])

const validClipboardNode = (node: unknown, budget: { remaining: number }, depth = 0): node is JSONContent => {
  if (!node || typeof node !== 'object' || depth > 32 || --budget.remaining < 0) return false
  const value = node as JSONContent
  if (typeof value.type !== 'string' || !allowedClipboardNodes.has(value.type)) return false
  if (value.text !== undefined && (typeof value.text !== 'string' || value.text.length > 1_000_000)) return false
  if (value.attrs !== undefined && (!value.attrs || typeof value.attrs !== 'object' || Array.isArray(value.attrs))) return false
  if (value.type === 'image' && !safeImageUrl(String(value.attrs?.src ?? ''))) return false
  if (value.marks !== undefined && (!Array.isArray(value.marks) || value.marks.some(mark => {
    if (!mark || typeof mark.type !== 'string' || !allowedClipboardMarks.has(mark.type)) return true
    return mark.type === 'link' && !safeUrl(String(mark.attrs?.href ?? ''))
  }))) return false
  return value.content === undefined || (Array.isArray(value.content)
    && value.content.every(child => validClipboardNode(child, budget, depth + 1)))
}

export const encodeInternalClipboard = (slice: Slice, schemaFingerprint = INTERNAL_SCHEMA_FINGERPRINT): string => JSON.stringify({
  version: INTERNAL_CLIPBOARD_VERSION,
  schema: 'semantic.prosemirror-slice',
  schemaFingerprint,
  slice: {
    ...slice.toJSON(),
    content: (slice.toJSON().content ?? []).map(stripCopiedIdentities),
  },
} satisfies ClipboardEnvelope)

export const decodeInternalClipboard = (
  raw: string,
  schema: Schema,
  schemaFingerprint = INTERNAL_SCHEMA_FINGERPRINT,
): Slice | null => {
  if (!raw || raw.length > 4_000_000) return null
  try {
    const value = JSON.parse(raw) as Partial<ClipboardEnvelope>
    if (value.version !== INTERNAL_CLIPBOARD_VERSION
      || value.schema !== 'semantic.prosemirror-slice'
      || value.schemaFingerprint !== schemaFingerprint
      || !value.slice || typeof value.slice !== 'object') return null
    const content = value.slice.content ?? []
    if (!Array.isArray(content) || !content.every(node => validClipboardNode(node, { remaining: 10_000 }))) return null
    const openStart = value.slice.openStart ?? 0
    const openEnd = value.slice.openEnd ?? 0
    if (!Number.isInteger(openStart) || !Number.isInteger(openEnd) || openStart < 0 || openEnd < 0) return null
    return Slice.fromJSON(schema, value.slice)
  } catch {
    return null
  }
}

export const parseTsvGrid = (text: string): string[][] | null => {
  if (!text.includes('\t') || text.length > 1_000_000) return null
  const rows: string[][] = [[]]
  let field = ''
  let quoted = false
  for (let index = 0; index < text.length; index += 1) {
    const char = text[index]
    if (char === '"') {
      if (quoted && text[index + 1] === '"') { field += '"'; index += 1 }
      else if (quoted || field.length === 0) quoted = !quoted
      else field += char
    } else if (char === '\t' && !quoted) {
      rows[rows.length - 1]?.push(field); field = ''
    } else if ((char === '\n' || char === '\r') && !quoted) {
      if (char === '\r' && text[index + 1] === '\n') index += 1
      rows[rows.length - 1]?.push(field); field = ''; rows.push([])
    } else field += char
  }
  if (quoted) return null
  rows[rows.length - 1]?.push(field)
  if (rows.length > 1 && rows[rows.length - 1]?.length === 1 && rows[rows.length - 1]?.[0] === '') rows.pop()
  const width = Math.max(0, ...rows.map(row => row.length))
  if (!width || rows.length > 100 || width > 100 || rows.length * width > 10_000) return null
  return rows.map(row => [...row, ...Array.from({ length: width - row.length }, () => '')])
}

const Mention = Node.create({
  name: 'semanticMention',
  group: 'inline',
  inline: true,
  atom: true,
  selectable: true,
  addAttributes() {
    return {
      semanticId: { default: null },
      entityId: { default: '' },
      label: { default: '' },
    }
  },
  parseHTML() { return [{ tag: 'span[data-semantic-mention]' }] },
  renderHTML({ node, HTMLAttributes }) {
    return ['span', mergeAttributes(HTMLAttributes, {
      'data-semantic-mention': node.attrs.entityId,
      class: 'dxeditor-engine__mention',
      contenteditable: 'false',
    }), `@${node.attrs.label}`]
  },
  renderText({ node }) { return `@${node.attrs.label}` },
})

const OpaqueBlock = Node.create({
  name: 'opaqueBlock',
  group: 'block',
  atom: true,
  selectable: true,
  isolating: true,
  addAttributes() {
    return {
      semanticId: { default: null },
      source: { default: '' },
      label: { default: 'Unsupported Markdown' },
      construct: { default: null },
      originalNode: { default: null, rendered: false },
    }
  },
  parseHTML() { return [{ tag: 'pre[data-semantic-opaque]' }] },
  renderHTML({ node, HTMLAttributes }) {
    return ['pre', mergeAttributes(HTMLAttributes, {
      'data-semantic-opaque': '',
      class: 'dxeditor-engine__opaque',
      contenteditable: 'false',
    }), ['code', {}, String(node.attrs.source)]]
  },
})

const OpaqueInline = Node.create({
  name: 'opaqueInline',
  group: 'inline',
  inline: true,
  atom: true,
  selectable: true,
  addAttributes() {
    return {
      semanticId: { default: null },
      source: { default: '' },
      construct: { default: null },
      originalNode: { default: null, rendered: false },
    }
  },
  parseHTML() { return [{ tag: 'code[data-semantic-opaque-inline]' }] },
  renderHTML({ node, HTMLAttributes }) {
    return ['code', mergeAttributes(HTMLAttributes, {
      'data-semantic-opaque-inline': '',
      class: 'dxeditor-engine__opaque-inline',
      contenteditable: 'false',
    }), String(node.attrs.source)]
  },
})

const SafeImage = Image.extend({
  addAttributes() {
    return {
      ...this.parent?.(),
      src: {
        default: null,
        parseHTML: element => {
          const src = element.getAttribute('src') ?? ''
          return safeImageUrl(src) ? src : null
        },
      },
    }
  },
})

const SafeLink = Link.extend({
  addAttributes() {
    return {
      ...this.parent?.(),
      title: {
        default: null,
        parseHTML: element => element.getAttribute('title'),
        renderHTML: attrs => attrs.title ? { title: String(attrs.title) } : {},
      },
    }
  },
})

const id = (() => {
  let next = 1
  return (prefix: string) => `${prefix}-${Date.now().toString(36)}-${next++}`
})()

const v1MarksToPm = (marks: MarkV1[]): JSONContent['marks'] => marks.flatMap(mark => {
  if (mark.component === 'bold' || mark.component === 'italic' || mark.component === 'code') {
    return [{ type: mark.component }]
  }
  if (mark.component === 'strike' || mark.component === 'strikethrough') {
    return [{ type: 'strike' }]
  }
  if (mark.component === 'link') {
    const href = String(mark.attrs.href ?? '')
    return safeUrl(href) ? [{ type: 'link', attrs: { href } }] : []
  }
  return []
})

const v1InlineToPm = (nodes: InlineV1[]): JSONContent[] => nodes.flatMap<JSONContent>(node => {
  if (node.component === 'mention') {
    return [{ type: 'semanticMention', attrs: { semanticId: node.id, entityId: node.attrs.id ?? '', label: node.text } }]
  }
  if (node.component === 'image') {
    const src = String(node.attrs.src ?? '')
    return safeImageUrl(src) ? [{ type: 'image', attrs: { semanticId: node.id, src, alt: node.text, title: node.attrs.title ?? null } }] : []
  }
  if (node.component === 'hard_break') return [{ type: 'hardBreak', attrs: { semanticId: node.id } }]
  if (node.component === 'raw_html') return [{ type: 'opaqueInline', attrs: { semanticId: node.id, source: node.text } }]
  if (node.text.length === 0) return []
  return [{ type: 'text', text: node.text, marks: v1MarksToPm(node.marks) }]
})

const contentValue = <T>(content: ContentV1, fallback: T): T => (content.value ?? fallback) as T
const pmInlineText = (nodes: InlineV1[]): string => nodes.map(node => node.text).join('')

const v1BlockToPm = (block: BlockV1): JSONContent => {
  const attrs = { ...block.attrs, semanticId: block.id }
  const inline = () => block.content.kind === 'Inline'
    ? v1InlineToPm(contentValue<InlineV1[]>(block.content, []))
    : []
  const children = () => block.content.kind === 'Blocks'
    ? contentValue<BlockV1[]>(block.content, []).map(v1BlockToPm)
    : []
  switch (block.component) {
    case 'heading': return { type: 'heading', attrs, content: inline() }
    case 'quote': return { type: 'blockquote', attrs, content: block.content.kind === 'Blocks' ? children() : [{ type: 'paragraph', content: inline() }] }
    case 'code': return { type: 'codeBlock', attrs: { ...attrs, language: block.attrs.info ?? block.attrs.language ?? null }, content: inline() }
    case 'divider': return { type: 'horizontalRule', attrs }
    case 'list': {
      const rawChildren = block.content.kind === 'Blocks' ? contentValue<BlockV1[]>(block.content, []) : []
      const task = block.attrs.task === true || rawChildren.some(item => Object.prototype.hasOwnProperty.call(item.attrs, 'checked'))
      const listType = task ? 'taskList' : block.attrs.ordered ? 'orderedList' : 'bulletList'
      return { type: listType, attrs, content: children() }
    }
    case 'list_item': {
      const itemType = Object.prototype.hasOwnProperty.call(block.attrs, 'checked') ? 'taskItem' : 'listItem'
      const itemChildren = children()
      return { type: itemType, attrs, content: itemChildren.length ? itemChildren : [{ type: 'paragraph' }] }
    }
    case 'table': {
      const table = contentValue<TableV1>(block.content, { rows: [] })
      return { type: 'table', attrs, content: table.rows.map((row, rowIndex) => ({
        type: 'tableRow', attrs: { ...row.attrs, semanticId: row.id }, content: row.cells.map(cell => ({
          type: rowIndex === 0 && cell.attrs.header === true ? 'tableHeader' : 'tableCell',
          attrs: { ...cell.attrs, semanticId: cell.id },
          content: cell.blocks.length ? cell.blocks.map(v1BlockToPm) : [{ type: 'paragraph' }],
        })),
      })) }
    }
    case 'raw_html': return { type: 'opaqueBlock', attrs: { ...attrs, source: pmInlineText(contentValue<InlineV1[]>(block.content, [])) } }
    case 'opaque_markdown': return { type: 'opaqueBlock', attrs: { ...attrs, source: block.attrs.source ?? '' } }
    default: return { type: 'paragraph', attrs, content: inline() }
  }
}

export const v1ToPm = (document: DocumentV1): JSONContent => ({
  type: 'doc',
  content: document.blocks.length ? document.blocks.map(v1BlockToPm) : [{ type: 'paragraph', attrs: { semanticId: id('block') } }],
})

const pmMarksToV1 = (marks: JSONContent['marks'] = []): MarkV1[] => marks.flatMap(mark => {
  if (['bold', 'italic', 'code'].includes(mark.type)) return [{ component: mark.type, attrs: {} }]
  if (mark.type === 'strike') return [{ component: 'strikethrough', attrs: {} }]
  if (mark.type === 'link' && safeUrl(String(mark.attrs?.href ?? ''))) {
    return [{ component: 'link', attrs: { href: mark.attrs?.href } }]
  }
  return []
})

const pmInlineToV1 = (nodes: JSONContent[] = []): InlineV1[] => nodes.flatMap<InlineV1>(node => {
  if (node.type === 'semanticMention') {
    return [{ id: node.attrs?.semanticId ?? id('mention'), component: 'mention', attrs: { id: node.attrs?.entityId ?? '' }, text: node.attrs?.label ?? '', marks: [] }]
  }
  if (node.type === 'image') {
    const src = String(node.attrs?.src ?? '')
    return safeImageUrl(src)
      ? [{ id: node.attrs?.semanticId ?? id('image'), component: 'image', attrs: { src, title: node.attrs?.title ?? null }, text: node.attrs?.alt ?? '', marks: [] }]
      : []
  }
  if (node.type === 'hardBreak') {
    return [{ id: node.attrs?.semanticId ?? id('hard-break'), component: 'hard_break', attrs: {}, text: '\n', marks: [] }]
  }
  if (node.type === 'opaqueInline') return [{ id: node.attrs?.semanticId ?? id('raw'), component: 'raw_html', attrs: {}, text: String(node.attrs?.source ?? ''), marks: [] }]
  return [{ id: id('text'), component: 'text', attrs: {}, text: node.text ?? '', marks: pmMarksToV1(node.marks) }]
}).filter(node => node.text.length > 0 || node.component !== 'text')

const inlineContent = (nodes: JSONContent[] = []): ContentV1 => ({ kind: 'Inline', value: pmInlineToV1(nodes) })

const pmBlockToV1 = (node: JSONContent): BlockV1 => {
  const attrs = { ...(node.attrs ?? {}) }
  const semanticId = String(attrs.semanticId ?? id('block'))
  delete attrs.semanticId
  switch (node.type) {
    case 'heading': return { id: semanticId, component: 'heading', attrs, content: inlineContent(node.content) }
    case 'blockquote': return { id: semanticId, component: 'quote', attrs, content: { kind: 'Blocks', value: (node.content ?? []).map(pmBlockToV1) } }
    case 'codeBlock': {
      if (attrs.language) attrs.info = attrs.language
      return { id: semanticId, component: 'code', attrs, content: inlineContent(node.content) }
    }
    case 'horizontalRule': return { id: semanticId, component: 'divider', attrs, content: { kind: 'Void' } }
    case 'bulletList':
    case 'orderedList':
    case 'taskList': return { id: semanticId, component: 'list', attrs: { ...attrs, ordered: node.type === 'orderedList', task: node.type === 'taskList' }, content: { kind: 'Blocks', value: (node.content ?? []).map(pmBlockToV1) } }
    case 'listItem':
    case 'taskItem': return { id: semanticId, component: 'list_item', attrs: { ...attrs, ...(node.type === 'taskItem' ? { checked: attrs.checked === true } : {}) }, content: { kind: 'Blocks', value: (node.content ?? []).map(pmBlockToV1) } }
    case 'table': return { id: semanticId, component: 'table', attrs, content: { kind: 'Table', value: {
      rows: (node.content ?? []).map(row => ({ id: String(row.attrs?.semanticId ?? id('row')), attrs: {}, cells: (row.content ?? []).map(cell => ({
        id: String(cell.attrs?.semanticId ?? id('cell')),
        attrs: { ...(cell.attrs ?? {}), header: cell.type === 'tableHeader' },
        blocks: (cell.content ?? []).map(pmBlockToV1),
      })) })),
    } } }
    case 'opaqueBlock': return { id: semanticId, component: 'raw_html', attrs: {}, content: { kind: 'Inline', value: [{ id: id('raw'), component: 'raw_html', attrs: {}, text: String(attrs.source ?? ''), marks: [] }] } }
    default: return { id: semanticId, component: 'paragraph', attrs, content: inlineContent(node.content) }
  }
}

export const pmToV1 = (document: JSONContent): DocumentV1 => ({
  schema: 'dxeditor.document.v1',
  blocks: (document.content ?? []).map(pmBlockToV1),
  meta: {},
})

const v2MarksToPm = (marks: ComponentMarkV2[] = []): JSONContent['marks'] => marks.flatMap(mark => {
  if (['bold', 'italic', 'strike', 'code'].includes(mark.kind)) return [{ type: mark.kind }]
  if (mark.kind === 'link') {
    const href = String(mark.attrs?.href ?? '')
    return safeUrl(href) ? [{ type: 'link', attrs: { href, title: mark.attrs?.title ?? null } }] : []
  }
  return []
})

const nodeAttrs = (node: ComponentNodeV2): Attributes => ({
  ...(node.attrs ?? {}),
  ...(node.id ? { semanticId: node.id } : {}),
})

const opaquePmNode = (node: ComponentNodeV2, inline: boolean): JSONContent => ({
  type: inline ? 'opaqueInline' : 'opaqueBlock',
  attrs: {
    semanticId: node.id,
    source: String(node.attrs?.source ?? node.attrs?.fallback ?? ''),
    construct: node.attrs?.construct ?? null,
    originalNode: node,
  },
})

const v2NodeToPm = (node: ComponentNodeV2, inlineContext = false): JSONContent => {
  const attrs = nodeAttrs(node)
  const content = () => (node.content ?? []).map(child => v2NodeToPm(child, false))
  const inline = () => (node.content ?? []).map(child => v2NodeToPm(child, true))
  switch (node.kind) {
    case 'text': return { type: 'text', text: node.text ?? '', marks: v2MarksToPm(node.marks) }
    case 'hard_break': return { type: 'hardBreak', attrs }
    case 'mention': return {
      type: 'semanticMention',
      attrs: { semanticId: node.id, entityId: node.attrs?.entity_id ?? '', label: node.attrs?.label ?? '' },
    }
    case 'image': {
      const src = String(node.attrs?.src ?? '')
      return safeImageUrl(src)
        ? { type: 'image', attrs: { semanticId: node.id, src, alt: node.attrs?.alt ?? '', title: node.attrs?.title ?? null } }
        : opaquePmNode(node, inlineContext)
    }
    case 'opaque_markdown_inline': return opaquePmNode(node, true)
    case 'paragraph': return { type: 'paragraph', attrs, content: inline() }
    case 'heading': return { type: 'heading', attrs: { ...attrs, level: Number(node.attrs?.level ?? 1) }, content: inline() }
    case 'blockquote': return { type: 'blockquote', attrs, content: content() }
    case 'code_block': return {
      type: 'codeBlock',
      attrs: { ...attrs, language: node.attrs?.info ?? null },
      content: inline(),
    }
    case 'thematic_break': return { type: 'horizontalRule', attrs }
    case 'bullet_list': return { type: 'bulletList', attrs, content: content() }
    case 'ordered_list': return { type: 'orderedList', attrs: { ...attrs, start: Number(node.attrs?.start ?? 1) }, content: content() }
    case 'task_list': return { type: 'taskList', attrs, content: content() }
    case 'list_item': return { type: 'listItem', attrs, content: content() }
    case 'task_item': return { type: 'taskItem', attrs: { ...attrs, checked: node.attrs?.checked === true }, content: content() }
    case 'table': return { type: 'table', attrs, content: content() }
    case 'table_row': return { type: 'tableRow', attrs, content: content() }
    case 'table_header': return { type: 'tableHeader', attrs, content: content() }
    case 'table_cell': return { type: 'tableCell', attrs, content: content() }
    case 'opaque_markdown_block': return opaquePmNode(node, false)
    default: return opaquePmNode(node, inlineContext)
  }
}

export const v2ToPm = (document: ComponentDocumentV2): JSONContent => {
  if (document.schema !== 'semantic.component-document' || document.version !== 2 || document.root.kind !== 'document') {
    throw new Error('unsupported component document schema')
  }
  const converted: JSONContent = {
    type: 'doc',
    content: document.root.content?.length
      ? document.root.content.map(node => v2NodeToPm(node, false))
      : [{ type: 'paragraph', attrs: { semanticId: id('paragraph') } }],
  }
  const identityIssue = validateSemanticIds(converted)
  if (identityIssue) throw new Error(identityIssue)
  return converted
}

const cleanAttrs = (attrs: Attributes = {}): Attributes => Object.fromEntries(
  Object.entries(attrs).filter(([key, value]) => key !== 'semanticId' && value !== null && value !== undefined),
)

const pmMarksToV2 = (marks: JSONContent['marks'] = []): ComponentMarkV2[] => marks.flatMap(mark => {
  if (['bold', 'italic', 'strike', 'code'].includes(mark.type)) return [{ kind: mark.type }]
  if (mark.type === 'link' && safeUrl(String(mark.attrs?.href ?? ''))) {
    return [{ kind: 'link', attrs: cleanAttrs({ href: mark.attrs?.href, title: mark.attrs?.title }) }]
  }
  return []
})

const originalOpaqueNode = (node: JSONContent): ComponentNodeV2 | null => {
  const original = node.attrs?.originalNode
  if (!original || typeof original !== 'object' || Array.isArray(original)) return null
  const value = original as ComponentNodeV2
  return typeof value.kind === 'string' ? value : null
}

const pmNodeToV2 = (node: JSONContent): ComponentNodeV2 => {
  const semanticId = (): string => {
    const value = typeof node.attrs?.semanticId === 'string' ? node.attrs.semanticId.trim() : ''
    if (!value) throw new Error(`node '${node.type}' is missing semanticId`)
    return value
  }
  const content = () => (node.content ?? []).map(pmNodeToV2)
  switch (node.type) {
    case 'text': return { kind: 'text', text: node.text ?? '', ...(node.marks?.length ? { marks: pmMarksToV2(node.marks) } : {}) }
    case 'hardBreak': return { kind: 'hard_break', id: semanticId() }
    case 'semanticMention': return {
      kind: 'mention', id: semanticId(),
      attrs: { entity_id: String(node.attrs?.entityId ?? ''), label: String(node.attrs?.label ?? '') },
    }
    case 'image': return {
      kind: 'image', id: semanticId(),
      attrs: cleanAttrs({ src: node.attrs?.src, alt: node.attrs?.alt ?? '', title: node.attrs?.title }),
    }
    case 'paragraph': return { kind: 'paragraph', id: semanticId(), content: content() }
    case 'heading': return { kind: 'heading', id: semanticId(), attrs: { level: Number(node.attrs?.level ?? 1) }, content: content() }
    case 'blockquote': return { kind: 'blockquote', id: semanticId(), content: content() }
    case 'codeBlock': return {
      kind: 'code_block', id: semanticId(),
      ...(node.attrs?.language ? { attrs: { info: String(node.attrs.language) } } : {}), content: content(),
    }
    case 'horizontalRule': return { kind: 'thematic_break', id: semanticId() }
    case 'bulletList': return { kind: 'bullet_list', id: semanticId(), content: content() }
    case 'orderedList': return { kind: 'ordered_list', id: semanticId(), attrs: { start: Number(node.attrs?.start ?? 1) }, content: content() }
    case 'taskList': return { kind: 'task_list', id: semanticId(), content: content() }
    case 'listItem': return { kind: 'list_item', id: semanticId(), content: content() }
    case 'taskItem': return { kind: 'task_item', id: semanticId(), attrs: { checked: node.attrs?.checked === true }, content: content() }
    case 'table': return { kind: 'table', id: semanticId(), content: content() }
    case 'tableRow': return { kind: 'table_row', id: semanticId(), content: content() }
    case 'tableHeader':
    case 'tableCell': return {
      kind: node.type === 'tableHeader' ? 'table_header' : 'table_cell', id: semanticId(),
      attrs: cleanAttrs({
        colspan: Number(node.attrs?.colspan ?? 1), rowspan: Number(node.attrs?.rowspan ?? 1),
        alignment: node.attrs?.alignment, colwidth: node.attrs?.colwidth,
      }),
      content: content(),
    }
    case 'opaqueInline': {
      const original = originalOpaqueNode(node)
      return original ? { ...original, id: semanticId() } : {
        kind: 'opaque_markdown_inline', id: semanticId(),
        attrs: cleanAttrs({ source: String(node.attrs?.source ?? ''), fallback: String(node.attrs?.source ?? ''), construct: node.attrs?.construct }),
      }
    }
    case 'opaqueBlock': {
      const original = originalOpaqueNode(node)
      return original ? { ...original, id: semanticId() } : {
        kind: 'opaque_markdown_block', id: semanticId(),
        attrs: cleanAttrs({ source: String(node.attrs?.source ?? ''), fallback: String(node.attrs?.source ?? ''), construct: node.attrs?.construct }),
      }
    }
    default: return { kind: 'unknown_component', id: semanticId(), attrs: { original_kind: node.type, fallback: '', payload: node } }
  }
}

export const pmToV2 = (document: JSONContent, metadata: Attributes = {}): ComponentDocumentV2 => {
  const identityIssue = validateSemanticIds(document)
  if (identityIssue) throw new Error(identityIssue)
  return {
    schema: 'semantic.component-document',
    version: 2,
    root: { kind: 'document', content: (document.content ?? []).map(pmNodeToV2) },
    ...(Object.keys(metadata).length ? { metadata } : {}),
  }
}

const button = (label: string, title: string, action: () => void): HTMLButtonElement => {
  const value = window.document.createElement('button')
  value.type = 'button'
  value.className = 'dxeditor-engine__button'
  value.textContent = label
  value.title = title
  value.setAttribute('aria-label', title)
  value.addEventListener('mousedown', event => event.preventDefault())
  value.addEventListener('click', action)
  return value
}

const positionSurface = (surface: HTMLElement, rect: DOMRect, wrapper: HTMLElement): void => {
  const bounds = wrapper.getBoundingClientRect()
  surface.style.left = `${Math.max(8, Math.min(rect.left - bounds.left, bounds.width - surface.offsetWidth - 8))}px`
  surface.style.top = `${Math.max(8, rect.top - bounds.top - surface.offsetHeight - 8)}px`
}

type TopLevelBlockTarget = { semanticId: string | null; fallbackPosition: number }

const positionAdjacentSurface = (surface: HTMLElement, rect: DOMRect, wrapper: HTMLElement): void => {
  const bounds = wrapper.getBoundingClientRect()
  const left = Math.max(8, Math.min(rect.right - bounds.left + 6, bounds.width - surface.offsetWidth - 8))
  const top = Math.max(8, Math.min(rect.top - bounds.top, bounds.height - surface.offsetHeight - 8))
  surface.style.left = `${left}px`
  surface.style.top = `${top}px`
}

export const mount = (host: HTMLElement, options: MountOptions): EditorSession => {
	  const knownAdapters = new Set([
	    'document', 'paragraph', 'heading', 'blockquote', 'bulletList', 'orderedList', 'taskList',
	    'listItem', 'taskItem', 'codeBlock', 'horizontalRule', 'table', 'tableRow', 'tableHeader',
	    'tableCell', 'text', 'hardBreak', 'image', 'semanticMention', 'opaqueBlock', 'opaqueInline',
	    'bold', 'italic', 'strike', 'code', 'link',
	  ])
	  if (options.manifest) {
	    if (options.manifest.version !== 1) throw new Error(`unsupported engine manifest version ${options.manifest.version}`)
	    if (options.schemaFingerprint && options.manifest.document_catalog_fingerprint !== options.schemaFingerprint) {
	      throw new Error('engine manifest catalog fingerprint does not match the mounted session')
	    }
	    const missing = options.manifest.components.find(component => !knownAdapters.has(component.adapter))
	    if (missing) throw new Error(`component '${missing.id}' requires missing behavior adapter '${missing.adapter}'`)
	  }
	  const commandManifest = new Map(options.manifest?.commands.map(command => [command.id, command]) ?? [])
	  const commandEnabled = (command: string): boolean => commandManifest.get(command)?.enabled ?? true
  const wrapper = host.closest<HTMLElement>('.dxeditor') ?? host.parentElement ?? host
  const overlays = wrapper.querySelector<HTMLElement>('[data-dxeditor-overlays]') ?? wrapper
  const clipboardSchemaFingerprint = options.manifest?.clipboard_schema_fingerprint
    ?? `${options.schemaFingerprint ?? 'standard'}:${INTERNAL_SCHEMA_FINGERPRINT}`
  const bubble = window.document.createElement('div')
  bubble.className = 'dxeditor-engine__surface dxeditor-engine__bubble'
  bubble.setAttribute('role', 'toolbar')
  bubble.setAttribute('aria-label', 'Text formatting')
  bubble.hidden = true
  const blockControls = window.document.createElement('div')
  blockControls.className = 'dxeditor-engine__surface dxeditor-engine__block-controls'
  blockControls.setAttribute('role', 'toolbar')
  blockControls.setAttribute('aria-label', 'Current block')
  blockControls.hidden = true
  const slash = window.document.createElement('div')
  slash.className = 'dxeditor-engine__surface dxeditor-engine__slash'
  slash.setAttribute('role', 'listbox')
  slash.setAttribute('aria-label', 'Insert block')
  slash.id = `dxeditor-slash-${options.sessionId}`
  slash.hidden = true
  const blockMenu = window.document.createElement('div')
  blockMenu.className = 'dxeditor-engine__surface dxeditor-engine__block-menu'
  blockMenu.setAttribute('role', 'menu')
  blockMenu.setAttribute('aria-label', 'Block actions')
  blockMenu.hidden = true
  const linkPopover = window.document.createElement('div')
  linkPopover.className = 'dxeditor-engine__surface dxeditor-engine__link-popover'
  linkPopover.setAttribute('role', 'dialog')
  linkPopover.setAttribute('aria-label', 'Edit link')
  linkPopover.hidden = true
  const linkLabel = window.document.createElement('label')
  linkLabel.textContent = 'Link URL'
  const linkInput = window.document.createElement('input')
  linkInput.type = 'url'
  linkInput.placeholder = 'https://example.com'
  linkInput.autocomplete = 'off'
  linkLabel.append(linkInput)
  const linkTitleLabel = window.document.createElement('label')
  linkTitleLabel.textContent = 'Link title'
  const linkTitleInput = window.document.createElement('input')
  linkTitleInput.type = 'text'
  linkTitleInput.autocomplete = 'off'
  linkTitleLabel.append(linkTitleInput)
  const linkError = window.document.createElement('span')
  linkError.className = 'dxeditor-engine__field-error'
  linkError.setAttribute('role', 'alert')
  const linkActions = window.document.createElement('div')
  linkActions.className = 'dxeditor-engine__link-actions'
  linkPopover.append(linkLabel, linkTitleLabel, linkError, linkActions)
  const tableControls = window.document.createElement('div')
  tableControls.className = 'dxeditor-engine__surface dxeditor-engine__table-controls'
  tableControls.setAttribute('role', 'toolbar')
  tableControls.setAttribute('aria-label', 'Table actions')
  tableControls.hidden = true
  const mediaPopover = window.document.createElement('div')
  mediaPopover.className = 'dxeditor-engine__surface dxeditor-engine__link-popover'
  mediaPopover.setAttribute('role', 'dialog')
  mediaPopover.setAttribute('aria-label', 'Image properties')
  mediaPopover.hidden = true
  const mediaInput = (label: string, type = 'text'): [HTMLLabelElement, HTMLInputElement] => {
    const wrapper = window.document.createElement('label')
    wrapper.textContent = label
    const input = window.document.createElement('input')
    input.type = type
    input.autocomplete = 'off'
    wrapper.append(input)
    return [wrapper, input]
  }
  const [mediaSourceLabel, mediaSource] = mediaInput('Image URL', 'url')
  const [mediaAltLabel, mediaAlt] = mediaInput('Alternative text')
  const [mediaTitleLabel, mediaTitle] = mediaInput('Title')
  mediaSource.placeholder = 'https://example.com/image.png'
  const mediaError = window.document.createElement('span')
  mediaError.className = 'dxeditor-engine__field-error'
  mediaError.setAttribute('role', 'alert')
  const mediaActions = window.document.createElement('div')
  mediaActions.className = 'dxeditor-engine__link-actions'
  mediaPopover.append(mediaSourceLabel, mediaAltLabel, mediaTitleLabel, mediaError, mediaActions)
  const mentions = window.document.createElement('div')
  mentions.className = 'dxeditor-engine__surface dxeditor-engine__slash'
  mentions.setAttribute('role', 'listbox')
  mentions.setAttribute('aria-label', 'Mention suggestions')
  mentions.id = `dxeditor-mentions-${options.sessionId}`
  mentions.hidden = true
  overlays.append(bubble, blockControls, slash, blockMenu, linkPopover, tableControls, mediaPopover, mentions)

	  let revision = 0
	  let lastEmittedSnapshot: string | null = null
  let destroyed = false
  let suppressUpdate = false
  let slashInsertionMode = false
  let slashBlockTarget: TopLevelBlockTarget | null = null
  let activeBlockTarget: TopLevelBlockTarget | null = null
  let blockTargetHovered = false
  let menuBlockTarget: TopLevelBlockTarget | null = null
  let blockMenuTrigger: HTMLButtonElement | null = null
  let pendingImageInsertAt: number | null = null
  let mentionRequest: AbortController | null = null
  let mentionQuery: string | null = null
	  let mentionTimer: number | undefined
	  let activeOptionIndex = 0
	  let dismissedSlash: string | null = null
	  let dismissedMention: string | null = null
	  let documentMetadata = options.document.metadata ?? {}
	  let lastCommandState = ''
	  const emitCommandState = (value: Editor): void => {
	    const state = JSON.stringify({ can_undo: value.can().undo(), can_redo: value.can().redo() })
	    if (state === lastCommandState) return
	    lastCommandState = state
	    options.emit({ kind: 'commandState', session_id: options.sessionId, ...JSON.parse(state) })
	  }
  const extensions = [
    Document, Paragraph, Text, Heading.configure({ levels: [1, 2, 3, 4, 5, 6] }), Bold, Italic, Strike, Code,
    Blockquote, CodeBlock, BulletList, OrderedList, ListItem, TaskList, TaskItem.configure({ nested: true }),
    HardBreak, HorizontalRule, SafeImage.configure({ allowBase64: false, inline: true }),
    SafeLink.configure({ openOnClick: false, autolink: true, defaultProtocol: 'https', protocols: ['http', 'https', 'mailto', 'tel'],
      isAllowedUri: url => safeUrl(url) }),
	    // Column widths and spans are not Markdown-persistable. Keep resize disabled until a
	    // typed format manifest explicitly enables it.
	    Table.configure({ resizable: options.manifest?.features.persistent_table_widths ?? false }), TableRow, TableHeader, TableCell,
    Mention, OpaqueBlock, OpaqueInline, SemanticId, CoreParagraphBehavior, Gapcursor, Dropcursor, UndoRedo,
  ]

  const editor = new Editor({
    element: host,
    extensions,
    content: v2ToPm(options.document),
    editable: !options.readonly,
    editorProps: {
	      attributes: { class: 'dxeditor-engine__content', 'aria-label': options.manifest?.aria_label ?? options.ariaLabel ?? 'Document editor', spellcheck: 'true' },
      transformPastedHTML: html => html.replace(/<(script|style|iframe|object|embed|form)[^>]*>[\s\S]*?<\/\1>/gi, ''),
      clipboardTextParser: (text, $context) => plainTextParagraphSlice(text, $context.doc.type.schema, $context.marks()),
      handleDOMEvents: {
        copy: (view, event) => writeClipboard(view, event as ClipboardEvent, false),
        cut: (view, event) => writeClipboard(view, event as ClipboardEvent, true),
      },
      handlePaste: (view, event) => {
        const internal = event.clipboardData?.getData(INTERNAL_CLIPBOARD_MIME) ?? ''
        const decoded = decodeInternalClipboard(internal, view.state.schema, clipboardSchemaFingerprint)
        if (decoded) {
          event.preventDefault()
          view.dispatch(view.state.tr.replaceSelection(decoded).scrollIntoView().setMeta('uiEvent', 'paste'))
          return true
        }
        const grid = parseTsvGrid(event.clipboardData?.getData('text/plain') ?? '')
        if (grid && pasteTableGrid(grid)) {
          event.preventDefault()
          return true
        }
        return false
      },
    },
	    onCreate: ({ editor }) => {
	      options.emit({ kind: 'ready', session_id: options.sessionId })
	      emitCommandState(editor)
	    },
	    onUpdate: () => { flush('transaction') },
	    onTransaction: ({ editor }) => { updateSurfaces(editor); emitCommandState(editor) },
	    onBlur: () => {
	      if (destroyed) return
	      flush('blur')
	      options.emit({ kind: 'blur', session_id: options.sessionId, revision, document: pmToV2(editor.getJSON(), documentMetadata) })
	    },
	  })
	  lastEmittedSnapshot = JSON.stringify(pmToV2(editor.getJSON(), documentMetadata))

	  function flush(reason: 'transaction' | 'blur' | 'replace' | 'destroy'): boolean {
	    if (suppressUpdate || destroyed) return false
	    try {
	      const document = pmToV2(editor.getJSON(), documentMetadata)
	      const serialized = JSON.stringify(document)
	      if (serialized === lastEmittedSnapshot) return false
	      revision += 1
	      lastEmittedSnapshot = serialized
	      options.emit({ kind: 'documentChange', session_id: options.sessionId, revision, document, reason })
	      return true
	    } catch (error) {
	      options.emit({ kind: 'error', session_id: options.sessionId, code: 'invalidSnapshot', message: String(error), revision, recoverable: true })
	      return false
	    }
	  }

  function writeClipboard(view: Editor['view'], event: ClipboardEvent, cut: boolean): boolean {
    const selection = view.state.selection
    if (selection.empty || !event.clipboardData) return false
    const slice = selection.content()
    const serialized = view.serializeForClipboard(slice)
    event.preventDefault()
    event.clipboardData.clearData()
    event.clipboardData.setData('text/html', serialized.dom.innerHTML)
    event.clipboardData.setData('text/plain', serialized.text)
    event.clipboardData.setData(INTERNAL_CLIPBOARD_MIME, encodeInternalClipboard(serialized.slice, clipboardSchemaFingerprint))
    if (cut) view.dispatch(view.state.tr.deleteSelection().scrollIntoView().setMeta('uiEvent', 'cut'))
    return true
  }

  const cellContent = (text: string): JSONContent[] => {
    const content: JSONContent[] = []
    text.split(/\r?\n/).forEach((line, index) => {
      if (index > 0) content.push({ type: 'hardBreak' })
      if (line) content.push({ type: 'text', text: line })
    })
    return [{ type: 'paragraph', attrs: { semanticId: id('paragraph') }, ...(content.length ? { content } : {}) }]
  }

  function pasteTableGrid(grid: string[][]): boolean {
    const { $from } = editor.state.selection
    let tableDepth = -1
    let rowDepth = -1
    for (let depth = $from.depth; depth > 0; depth -= 1) {
      const type = $from.node(depth).type.name
      if (rowDepth < 0 && type === 'tableRow') rowDepth = depth
      if (type === 'table') { tableDepth = depth; break }
    }
    if (tableDepth < 0 || rowDepth < 0) return false
    const table = $from.node(tableDepth)
    const rowIndex = $from.index(tableDepth)
    const cellIndex = $from.index(rowDepth)
    const tableJson = table.toJSON() as JSONContent
    const rows = tableJson.content ?? []
    if (!rows.length || rows.some(row => !row.content?.length
      || row.content.some(cell => Number(cell.attrs?.colspan ?? 1) !== 1 || Number(cell.attrs?.rowspan ?? 1) !== 1))) return false
    const currentWidth = Math.max(...rows.map(row => row.content?.length ?? 0))
    const requiredRows = rowIndex + grid.length
    const requiredWidth = cellIndex + grid[0].length
    while (rows.length < requiredRows) {
      rows.push({
        type: 'tableRow', attrs: { semanticId: id('row') },
        content: Array.from({ length: Math.max(currentWidth, requiredWidth) }, () => ({
          type: 'tableCell', attrs: { semanticId: id('cell') }, content: cellContent(''),
        })),
      })
    }
    rows.forEach((row, index) => {
      row.content ??= []
      while (row.content.length < requiredWidth) {
        const header = index === 0 && row.content.every(cell => cell.type === 'tableHeader')
        row.content.push({
          type: header ? 'tableHeader' : 'tableCell',
          attrs: { semanticId: id('cell') }, content: cellContent(''),
        })
      }
    })
    grid.forEach((values, rowOffset) => values.forEach((text, columnOffset) => {
      const cell = rows[rowIndex + rowOffset].content?.[cellIndex + columnOffset]
      if (cell) cell.content = cellContent(text)
    }))
    try {
      const replacement = editor.schema.nodeFromJSON(tableJson)
      const tablePosition = $from.before(tableDepth)
      editor.view.dispatch(editor.state.tr.replaceWith(tablePosition, tablePosition + table.nodeSize, replacement).scrollIntoView().setMeta('uiEvent', 'paste'))
      return true
    } catch {
      return false
    }
  }

  const run = (name: string, attrs: Attributes = {}): boolean => {
	    if (!commandEnabled(name)) return false
    const chain = editor.chain().focus()
    switch (name) {
      case 'undo': return chain.undo().run()
      case 'redo': return chain.redo().run()
      case 'bold': return chain.toggleBold().run()
      case 'italic': return chain.toggleItalic().run()
      case 'strike': return chain.toggleStrike().run()
      case 'code': return chain.toggleCode().run()
      case 'paragraph': return chain.setParagraph().run()
      case 'heading': return chain.toggleHeading({ level: Number(attrs.level ?? 1) as 1 | 2 | 3 | 4 | 5 | 6 }).run()
      case 'blockquote': return chain.toggleBlockquote().run()
      case 'codeBlock': return chain.toggleCodeBlock().run()
      case 'bulletList': return chain.toggleBulletList().run()
      case 'orderedList': return chain.toggleOrderedList().run()
      case 'taskList': return chain.toggleTaskList().run()
      case 'horizontalRule': return chain.setHorizontalRule().run()
      case 'table': return chain.insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run()
      case 'addRowBefore': return chain.addRowBefore().run()
      case 'addRowAfter': return chain.addRowAfter().run()
      case 'deleteRow': return chain.deleteRow().run()
      case 'addColumnBefore': return chain.addColumnBefore().run()
      case 'addColumnAfter': return chain.addColumnAfter().run()
      case 'deleteColumn': return chain.deleteColumn().run()
      case 'toggleHeaderRow': return chain.toggleHeaderRow().run()
      case 'alignCell': {
        const alignment = String(attrs.alignment ?? '')
        return ['left', 'center', 'right'].includes(alignment) && chain.setCellAttribute('alignment', alignment).run()
      }
      case 'deleteTable': return chain.deleteTable().run()
      case 'image': {
        const src = String(attrs.src ?? '')
        if (!safeImageUrl(src)) return false
        return chain.setImage({ src, alt: String(attrs.alt ?? ''), title: String(attrs.title ?? '') || undefined }).run()
      }
      case 'link': {
        const href = String(attrs.href ?? '')
        const title = String(attrs.title ?? '') || null
        return href === '' ? chain.unsetLink().run() : safeUrl(href) && chain.extendMarkRange('link').setLink({ href, title }).run()
      }
      default: return false
    }
  }

  type TopLevelBlockRange = { from: number; to: number; index: number; target: TopLevelBlockTarget }

  const topLevelRange = (target: TopLevelBlockTarget | null = null): TopLevelBlockRange | null => {
    let fallback: TopLevelBlockRange | null = null
    let identityMatch: TopLevelBlockRange | null = null
    const selectionPosition = editor.state.selection.$from.pos
    editor.state.doc.forEach((node, offset, index) => {
      const semanticId = typeof node.attrs.semanticId === 'string' ? node.attrs.semanticId : null
      const range = {
        from: offset,
        to: offset + node.nodeSize,
        index,
        target: { semanticId, fallbackPosition: offset },
      }
      if (target?.semanticId && semanticId === target.semanticId) identityMatch = range
      const position = target?.fallbackPosition ?? selectionPosition
      if (position >= offset && position <= offset + node.nodeSize) fallback = range
    })
    // Never redirect an identity-backed action to the node that merely inherited its old
    // position after an external edit. A vanished target should make the action a no-op.
    return target?.semanticId ? identityMatch : fallback
  }

  const targetFromDom = (domTarget: EventTarget | null): TopLevelBlockTarget | null => {
    if (!(domTarget instanceof globalThis.Node)) return null
    let match: TopLevelBlockTarget | null = null
    editor.state.doc.forEach((node, offset) => {
      if (match) return
      const dom = editor.view.nodeDOM(offset)
      if (!(dom instanceof HTMLElement) || (dom !== domTarget && !dom.contains(domTarget))) return
      match = {
        semanticId: typeof node.attrs.semanticId === 'string' ? node.attrs.semanticId : null,
        fallbackPosition: offset,
      }
    })
    return match
  }

  const deleteBlock = (target: TopLevelBlockTarget | null): void => {
    const range = topLevelRange(target)
    if (range) editor.chain().focus().deleteRange(range).run()
  }

  const duplicateBlock = (target: TopLevelBlockTarget | null): void => {
    const range = topLevelRange(target)
    if (!range) return
    const source = editor.state.doc.nodeAt(range.from)
    if (!source) return
    const regenerate = (content: JSONContent): JSONContent => ({
      ...content,
      attrs: content.attrs ? { ...content.attrs, semanticId: id(content.type ?? 'node') } : content.attrs,
      content: content.content?.map(regenerate),
    })
    const copy = editor.schema.nodeFromJSON(regenerate(source.toJSON()))
    editor.view.dispatch(editor.state.tr.insert(range.to, copy).scrollIntoView())
    editor.commands.focus(range.to + 1)
  }

  const moveBlock = (target: TopLevelBlockTarget | null, direction: -1 | 1): void => {
    const range = topLevelRange(target)
    if (!range) return
    const { index } = range
    const source = editor.state.doc.child(range.index)
    if (direction < 0) {
      if (index === 0) return
      const destination = range.from - editor.state.doc.child(index - 1).nodeSize
      editor.view.dispatch(editor.state.tr.delete(range.from, range.to).insert(destination, source).scrollIntoView())
      editor.commands.focus(destination + 1)
    } else {
      if (index >= editor.state.doc.childCount - 1) return
      const destination = range.from + editor.state.doc.child(index + 1).nodeSize
      editor.view.dispatch(editor.state.tr.delete(range.from, range.to).insert(destination, source).scrollIntoView())
      editor.commands.focus(destination + 1)
    }
  }

  const removeSlashTrigger = (): void => {
    const { $from } = editor.state.selection
    const textBefore = $from.parent.textBetween(0, $from.parentOffset, undefined, '\ufffc')
    const match = /(?:^|\s)(\/[^\s/]*)$/.exec(textBefore)
    if (match) editor.commands.deleteRange({ from: $from.pos - match[1].length, to: $from.pos })
  }

  const insertBlockAfter = (name: string, attrs: Attributes = {}, target: TopLevelBlockTarget | null = null): void => {
    const range = topLevelRange(target)
    if (!range) return
    if (name === 'table') {
      const rows: JSONContent[] = Array.from({ length: 3 }, (_, row) => ({
        type: 'tableRow', attrs: { semanticId: id('row') },
        content: Array.from({ length: 3 }, () => ({
          type: row === 0 ? 'tableHeader' : 'tableCell',
          attrs: { semanticId: id('cell') },
          content: [{ type: 'paragraph', attrs: { semanticId: id('paragraph') } }],
        })),
      }))
      editor.chain().focus().insertContentAt(range.to, { type: 'table', attrs: { semanticId: id('table') }, content: rows }).run()
      return
    }
    const paragraph = { type: 'paragraph', attrs: { semanticId: id('paragraph') } }
    const blocks: Record<string, JSONContent> = {
      paragraph,
      heading: { type: 'heading', attrs: { semanticId: id('heading'), level: Number(attrs.level ?? 1) } },
      blockquote: { type: 'blockquote', attrs: { semanticId: id('quote') }, content: [paragraph] },
      codeBlock: { type: 'codeBlock', attrs: { semanticId: id('code') } },
      horizontalRule: { type: 'horizontalRule', attrs: { semanticId: id('divider') } },
      bulletList: { type: 'bulletList', attrs: { semanticId: id('list') }, content: [{ type: 'listItem', attrs: { semanticId: id('item') }, content: [paragraph] }] },
      orderedList: { type: 'orderedList', attrs: { semanticId: id('list'), start: 1 }, content: [{ type: 'listItem', attrs: { semanticId: id('item') }, content: [paragraph] }] },
      taskList: { type: 'taskList', attrs: { semanticId: id('list') }, content: [{ type: 'taskItem', attrs: { semanticId: id('item'), checked: false }, content: [paragraph] }] },
    }
    const block = blocks[name]
    if (block) editor.chain().focus().insertContentAt(range.to, block).run()
  }

  ;[['B', 'Bold', 'bold'], ['I', 'Italic', 'italic'], ['S', 'Strikethrough', 'strike'], ['<>', 'Inline code', 'code']]
	    .filter(([, , command]) => commandEnabled(command))
	    .forEach(([label, title, command]) => {
	      const control = button(label, title, () => run(command))
	      control.dataset.command = command
	      control.setAttribute('aria-pressed', 'false')
	      bubble.append(control)
	    })
  if (commandEnabled('link')) {
    bubble.append(button('Link', 'Add or edit link', () => {
      const link = editor.getAttributes('link')
      linkInput.value = String(link.href ?? '')
      linkTitleInput.value = String(link.title ?? '')
      linkError.textContent = ''
      linkPopover.hidden = false
      positionSurface(linkPopover, bubble.getBoundingClientRect(), wrapper)
      linkInput.focus()
      linkInput.select()
    }))
  }
  const applyLink = (): void => {
    const href = linkInput.value.trim()
    if (href && !safeUrl(href)) {
      linkError.textContent = 'Use an HTTP, HTTPS, mail, telephone, or relative URL.'
      return
    }
    run('link', { href, title: linkTitleInput.value.trim() })
    linkPopover.hidden = true
  }
  linkActions.append(button('Apply', 'Apply link', applyLink))
  linkActions.append(button('Remove', 'Remove link', () => { run('link', { href: '' }); linkPopover.hidden = true }))
  linkActions.append(button('Open', 'Open link', () => {
    const href = linkInput.value.trim()
    if (safeUrl(href)) window.open(href, '_blank', 'noopener,noreferrer')
  }))
  ;[linkInput, linkTitleInput].forEach(input => input.addEventListener('keydown', event => {
    if (event.key === 'Enter') { event.preventDefault(); applyLink() }
    if (event.key === 'Escape') { linkPopover.hidden = true; editor.commands.focus() }
  }))
  const openMediaPopover = (insertAt: number | null): void => {
    pendingImageInsertAt = insertAt
    const selected = editor.isActive('image') ? editor.getAttributes('image') : {}
    mediaSource.value = String(selected.src ?? '')
    mediaAlt.value = String(selected.alt ?? '')
    mediaTitle.value = String(selected.title ?? '')
    mediaError.textContent = ''
    mediaPopover.hidden = false
    const anchor = editor.view.coordsAtPos(editor.state.selection.from)
    positionSurface(mediaPopover, new DOMRect(anchor.left, anchor.bottom, 1, 1), wrapper)
    mediaSource.focus()
  }
  const applyMedia = (): void => {
    const src = mediaSource.value.trim()
    if (!safeImageUrl(src)) {
      mediaError.textContent = 'Use an HTTP or HTTPS image URL.'
      return
    }
    const attrs = { src, alt: mediaAlt.value.trim(), title: mediaTitle.value.trim() || null, semanticId: id('image') }
    if (pendingImageInsertAt !== null) {
      editor.chain().focus().insertContentAt(pendingImageInsertAt, {
        type: 'paragraph', attrs: { semanticId: id('paragraph') }, content: [{ type: 'image', attrs }],
      }).run()
    } else if (editor.isActive('image')) editor.chain().focus().updateAttributes('image', attrs).run()
    else editor.chain().focus().insertContent({ type: 'image', attrs }).run()
    pendingImageInsertAt = null
    mediaPopover.hidden = true
  }
  mediaActions.append(button('Apply', 'Apply image properties', applyMedia))
  mediaActions.append(button('Remove', 'Remove image', () => {
    if (editor.isActive('image')) editor.chain().focus().deleteSelection().run()
    pendingImageInsertAt = null
    mediaPopover.hidden = true
  }))
  ;[mediaSource, mediaAlt, mediaTitle].forEach(input => input.addEventListener('keydown', event => {
    if (event.key === 'Enter') { event.preventDefault(); applyMedia() }
    if (event.key === 'Escape') { pendingImageInsertAt = null; mediaPopover.hidden = true; editor.commands.focus() }
  }))
  const addBlockButton = button('+', 'Add a block', () => {
    slashBlockTarget = activeBlockTarget
    slashInsertionMode = true
    slash.hidden = false
    positionAdjacentSurface(slash, blockControls.getBoundingClientRect(), wrapper)
    slash.querySelector<HTMLButtonElement>('button')?.focus()
  })
  const blockActionsButton = button('⋮⋮', 'Block actions', () => {
    menuBlockTarget = activeBlockTarget
    blockMenuTrigger = blockActionsButton
    blockMenu.hidden = false
    const range = topLevelRange(menuBlockTarget)
    moveUpButton.disabled = !range || range.index === 0
    moveDownButton.disabled = !range || range.index === editor.state.doc.childCount - 1
    positionAdjacentSurface(blockMenu, blockControls.getBoundingClientRect(), wrapper)
    blockMenu.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus()
  })
  blockControls.append(addBlockButton, blockActionsButton)
  const duplicateButton = button('Duplicate', 'Duplicate block', () => { duplicateBlock(menuBlockTarget); blockMenu.hidden = true })
  const moveUpButton = button('Move up', 'Move block up', () => { moveBlock(menuBlockTarget, -1); blockMenu.hidden = true })
  const moveDownButton = button('Move down', 'Move block down', () => { moveBlock(menuBlockTarget, 1); blockMenu.hidden = true })
  const deleteButton = button('Delete', 'Delete block', () => { deleteBlock(menuBlockTarget); blockMenu.hidden = true })
  blockMenu.append(duplicateButton, moveUpButton, moveDownButton, deleteButton)
	  blockMenu.querySelectorAll('button').forEach(item => item.setAttribute('role', 'menuitem'))
  blockMenu.addEventListener('keydown', event => {
    const items = Array.from(blockMenu.querySelectorAll<HTMLButtonElement>('button:not(:disabled)'))
    const index = Math.max(0, items.indexOf(window.document.activeElement as HTMLButtonElement))
    if (event.key === 'Escape') {
      event.preventDefault()
      blockMenu.hidden = true
      blockMenuTrigger?.focus()
    } else if (event.key === 'ArrowDown' || event.key === 'ArrowUp' || event.key === 'Home' || event.key === 'End') {
      event.preventDefault()
      const next = event.key === 'Home' ? 0
        : event.key === 'End' ? items.length - 1
          : (index + (event.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length
      items[next]?.focus()
    }
  })
  const insertions: Array<[string, string, Attributes?]> = [
    ['Text', 'paragraph'], ['Heading 1', 'heading', { level: 1 }], ['Heading 2', 'heading', { level: 2 }],
    ['Bulleted list', 'bulletList'], ['Numbered list', 'orderedList'], ['Task list', 'taskList'],
    ['Quote', 'blockquote'], ['Code block', 'codeBlock'], ['Divider', 'horizontalRule'], ['Table', 'table'], ['Image', 'image'],
  ]
  insertions.filter(([, command]) => commandEnabled(command)).forEach(([label, command, attrs]) => {
    const item = button(label, `Insert ${label}`, () => {
      if (command === 'image') {
        const position = slashInsertionMode ? topLevelRange(slashBlockTarget)?.to ?? null : null
        if (!slashInsertionMode) removeSlashTrigger()
        openMediaPopover(position)
      } else if (slashInsertionMode) insertBlockAfter(command, attrs, slashBlockTarget)
      else { removeSlashTrigger(); run(command, attrs) }
      slash.hidden = true
      slashInsertionMode = false
      slashBlockTarget = null
    })
    item.setAttribute('role', 'option')
    slash.append(item)
  })
  ;[['+ Row', 'Add row after', 'addRowAfter'], ['− Row', 'Delete row', 'deleteRow'], ['+ Column', 'Add column after', 'addColumnAfter'],
    ['− Column', 'Delete column', 'deleteColumn'], ['Header', 'Toggle header row', 'toggleHeaderRow'],
    ['⇤', 'Align cell left', 'alignCell', 'left'], ['↔', 'Align cell center', 'alignCell', 'center'], ['⇥', 'Align cell right', 'alignCell', 'right'],
    ['Delete', 'Delete table', 'deleteTable']]
	    .filter(([, , command]) => commandEnabled(command))
	    .forEach(([label, title, command, alignment]) => tableControls.append(button(label, title, () => run(command, alignment ? { alignment } : {}))))

  const currentMention = (): { from: number; to: number; query: string } | null => {
    const { $from } = editor.state.selection
    if (!editor.state.selection.empty || !$from.parent.isTextblock) return null
    const before = $from.parent.textBetween(0, $from.parentOffset, undefined, '\ufffc')
    const match = /(?:^|\s)@([^\s@]{0,64})$/.exec(before)
    return match ? { from: $from.pos - match[1].length - 1, to: $from.pos, query: match[1] } : null
  }

	  const mentionKey = (range: { from: number; to: number; query: string }): string =>
	    `${range.from}:${range.to}:${range.query}`

	  const syncListbox = (surface: HTMLElement, reset = false): void => {
	    const items = Array.from(surface.querySelectorAll<HTMLButtonElement>('button[role="option"]'))
	      .filter(item => !item.hidden && !item.disabled)
	    if (reset) activeOptionIndex = 0
	    activeOptionIndex = Math.max(0, Math.min(activeOptionIndex, Math.max(0, items.length - 1)))
	    items.forEach((item, index) => {
	      item.id ||= `${surface.id}-option-${index}`
	      item.tabIndex = -1
	      item.setAttribute('aria-selected', String(index === activeOptionIndex))
	    })
	    editor.view.dom.setAttribute('aria-expanded', String(items.length > 0))
	    editor.view.dom.setAttribute('aria-controls', surface.id)
	    const active = items[activeOptionIndex]
	    if (active) editor.view.dom.setAttribute('aria-activedescendant', active.id)
	    else editor.view.dom.removeAttribute('aria-activedescendant')
	  }

  const closeMentions = (): void => {
    window.clearTimeout(mentionTimer)
    mentionRequest?.abort()
    mentionRequest = null
    mentionQuery = null
    mentions.hidden = true
    mentions.replaceChildren()
  }

  const updateMentions = (): void => {
    const match = options.mentionProvider ? currentMention() : null
    if (!match) { closeMentions(); return }
	    if (dismissedMention === mentionKey(match)) { closeMentions(); return }
	    if (dismissedMention) dismissedMention = null
    const caret = editor.view.coordsAtPos(match.to)
    mentions.hidden = false
    positionSurface(mentions, new DOMRect(caret.left, caret.bottom, 1, 1), wrapper)
    if (match.query === mentionQuery) return
    mentionQuery = match.query
    window.clearTimeout(mentionTimer)
    mentionRequest?.abort()
    mentionRequest = new AbortController()
    const request = mentionRequest
    mentions.replaceChildren()
    const loading = window.document.createElement('span')
    loading.textContent = 'Searching…'
    loading.setAttribute('role', 'status')
    mentions.append(loading)
    mentionTimer = window.setTimeout(async () => {
      try {
        const candidates = await options.mentionProvider?.(match.query, { signal: request.signal, sessionId: options.sessionId }) ?? []
        if (request.signal.aborted || mentionQuery !== match.query) return
        mentions.replaceChildren()
        candidates.slice(0, 12).forEach(candidate => {
          if (!candidate || typeof candidate.id !== 'string' || typeof candidate.label !== 'string') return
          const item = button(candidate.detail ? `${candidate.label} — ${candidate.detail}` : candidate.label, `Mention ${candidate.label}`, () => {
            const active = currentMention()
            if (!active) return
            editor.chain().focus().deleteRange({ from: active.from, to: active.to }).insertContent([
              { type: 'semanticMention', attrs: { semanticId: id('mention'), entityId: candidate.id, label: candidate.label } },
              { type: 'text', text: ' ' },
            ]).run()
            closeMentions()
          })
          item.setAttribute('role', 'option')
          mentions.append(item)
        })
        if (!mentions.childElementCount) {
          const empty = window.document.createElement('span')
          empty.textContent = 'No matches'
          empty.setAttribute('role', 'status')
          mentions.append(empty)
        }
	        syncListbox(mentions, true)
      } catch (error) {
        if (request.signal.aborted) return
        mentions.replaceChildren()
        const failure = window.document.createElement('span')
        failure.textContent = 'Suggestions unavailable'
        failure.setAttribute('role', 'status')
        mentions.append(failure)
      }
    }, 120)
  }

  const positionBlockControls = (): void => {
    if (options.readonly) {
      blockControls.hidden = true
      return
    }
    const range = topLevelRange(activeBlockTarget)
    if (!range) {
      blockControls.hidden = true
      return
    }
    activeBlockTarget = range.target
    const dom = editor.view.nodeDOM(range.from)
    if (!(dom instanceof HTMLElement)) {
      blockControls.hidden = true
      return
    }
    const rect = dom.getBoundingClientRect()
    const viewportHeight = window.visualViewport?.height ?? window.innerHeight
    if (rect.bottom < 0 || rect.top > viewportHeight) {
      blockControls.hidden = true
      return
    }
    blockControls.hidden = false
    const bounds = wrapper.getBoundingClientRect()
    blockControls.style.left = `${rect.left - bounds.left - blockControls.offsetWidth - 6}px`
    blockControls.style.top = `${rect.top - bounds.top}px`
    blockControls.dataset.blockId = range.target.semanticId ?? ''
  }

  function updateSurfaces(value: Editor): void {
    if (destroyed || options.readonly || value.view.composing) return
    const { from, to, empty } = value.state.selection
    const cellSelection = value.state.selection instanceof CellSelection
    const imageActive = value.isActive('image')
	    bubble.querySelectorAll<HTMLButtonElement>('[data-command]').forEach(control => {
	      control.setAttribute('aria-pressed', String(value.isActive(control.dataset.command ?? '')))
    })
    bubble.hidden = empty || imageActive || cellSelection
    if (!blockTargetHovered && blockMenu.hidden) activeBlockTarget = topLevelRange()?.target ?? null
    positionBlockControls()
    tableControls.hidden = !value.isActive('table')
    if (!bubble.hidden) {
      const start = value.view.coordsAtPos(from)
      const end = value.view.coordsAtPos(to)
      positionSurface(bubble, new DOMRect(Math.min(start.left, end.left), Math.min(start.top, end.top), Math.abs(end.right - start.left), Math.max(start.bottom, end.bottom) - Math.min(start.top, end.top)), wrapper)
    }
    if (!tableControls.hidden) {
      const caret = value.view.coordsAtPos(from)
      positionSurface(tableControls, new DOMRect(caret.left, caret.top, 1, caret.bottom - caret.top), wrapper)
    }
    if (imageActive) {
      const caret = value.view.coordsAtPos(from)
      mediaPopover.hidden = false
      positionSurface(mediaPopover, new DOMRect(caret.left, caret.bottom, 1, 1), wrapper)
      if (!mediaPopover.contains(window.document.activeElement)) {
        pendingImageInsertAt = null
        const attrs = value.getAttributes('image')
        mediaSource.value = String(attrs.src ?? '')
        mediaAlt.value = String(attrs.alt ?? '')
        mediaTitle.value = String(attrs.title ?? '')
        mediaError.textContent = ''
      }
    } else if (pendingImageInsertAt === null && !mediaPopover.contains(window.document.activeElement)) {
      mediaPopover.hidden = true
    }
    const parent = value.state.selection.$from.parent
    const textBefore = parent.textBetween(0, value.state.selection.$from.parentOffset, undefined, '\ufffc')
    const slashMatch = /(?:^|\s)\/([^\s/]*)$/.exec(textBefore)
    if (slashMatch && empty) {
	      const slashKey = `${from}:${slashMatch[1]}`
	      if (dismissedSlash === slashKey) {
	        slash.hidden = true
	        updateMentions()
	        return
	      }
	      if (dismissedSlash) dismissedSlash = null
      slashInsertionMode = false
      slash.hidden = false
      const caret = value.view.coordsAtPos(from)
      positionSurface(slash, new DOMRect(caret.left, caret.bottom, 1, 1), wrapper)
      const query = slashMatch[1].toLocaleLowerCase()
      slash.querySelectorAll<HTMLButtonElement>('button').forEach(item => { item.hidden = !item.textContent?.toLocaleLowerCase().includes(query) })
	      syncListbox(slash, true)
    } else if (!slashInsertionMode && window.document.activeElement && !slash.contains(window.document.activeElement)) {
      slash.hidden = true
    }
    updateMentions()
  }

  const blockPointerMove = (event: PointerEvent): void => {
    if (options.readonly) return
    const target = targetFromDom(event.target)
    if (!target) return
    blockTargetHovered = true
    activeBlockTarget = target
    positionBlockControls()
  }
  const blockPointerLeave = (event: PointerEvent): void => {
    const related = event.relatedTarget
    if (related instanceof globalThis.Node && (blockControls.contains(related) || blockMenu.contains(related))) return
    blockTargetHovered = false
    if (blockMenu.hidden) activeBlockTarget = topLevelRange()?.target ?? null
    positionBlockControls()
  }
  editor.view.dom.addEventListener('pointermove', blockPointerMove)
  editor.view.dom.addEventListener('pointerleave', blockPointerLeave)

  const outside = (event: PointerEvent) => {
    const target = event.target as globalThis.Node
    if (![bubble, slash, blockControls, blockMenu, linkPopover, tableControls, mediaPopover, mentions].some(surface => surface.contains(target))) {
      slash.hidden = true
      slashInsertionMode = false
      slashBlockTarget = null
      blockMenu.hidden = true
      menuBlockTarget = null
      linkPopover.hidden = true
      if (!editor.isActive('image')) mediaPopover.hidden = true
      closeMentions()
    }
  }
  window.document.addEventListener('pointerdown', outside, true)
  let geometryFrame: number | null = null
  const refreshGeometry = (): void => {
    if (geometryFrame !== null || destroyed) return
    geometryFrame = window.requestAnimationFrame(() => {
      geometryFrame = null
      if ([bubble, slash, blockControls, blockMenu, linkPopover, tableControls, mediaPopover, mentions]
        .some(surface => !surface.hidden)) updateSurfaces(editor)
    })
  }
  window.addEventListener('scroll', refreshGeometry, true)
  window.addEventListener('resize', refreshGeometry)
  window.visualViewport?.addEventListener('resize', refreshGeometry)
  window.visualViewport?.addEventListener('scroll', refreshGeometry)
	  const geometryObserver = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(refreshGeometry)
	  geometryObserver?.observe(editor.view.dom)
	  const editorKeydown = (event: KeyboardEvent): void => {
	    const activeListbox = !mentions.hidden ? mentions : !slash.hidden ? slash : null
	    const items = activeListbox
	      ? Array.from(activeListbox.querySelectorAll<HTMLButtonElement>('button[role="option"]')).filter(item => !item.hidden && !item.disabled)
	      : []
	    if (activeListbox && items.length && ['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
	      event.preventDefault()
	      event.stopPropagation()
	      if (event.key === 'Home') activeOptionIndex = 0
	      else if (event.key === 'End') activeOptionIndex = items.length - 1
	      else activeOptionIndex = (activeOptionIndex + (event.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length
	      syncListbox(activeListbox)
	      return
	    }
	    if (activeListbox && items.length && event.key === 'Enter' && !event.isComposing) {
	      event.preventDefault()
	      event.stopPropagation()
	      items[activeOptionIndex]?.click()
	      return
	    }
    if (event.key === 'Escape') {
	      const mention = currentMention()
	      if (!mentions.hidden && mention) dismissedMention = mentionKey(mention)
	      const { $from } = editor.state.selection
	      const before = $from.parent.textBetween(0, $from.parentOffset, undefined, '\ufffc')
	      const slashMatch = /(?:^|\s)\/([^\s/]*)$/.exec(before)
	      if (!slash.hidden && slashMatch) dismissedSlash = `${$from.pos}:${slashMatch[1]}`
      slash.hidden = true; slashBlockTarget = null; blockMenu.hidden = true; menuBlockTarget = null; linkPopover.hidden = true; mediaPopover.hidden = true
      bubble.hidden = true; tableControls.hidden = true; pendingImageInsertAt = null; closeMentions(); editor.commands.focus()
	      editor.view.dom.setAttribute('aria-expanded', 'false')
	      editor.view.dom.removeAttribute('aria-activedescendant')
    }
	  }
	  editor.view.dom.addEventListener('keydown', editorKeydown, true)
  updateSurfaces(editor)

  return {
    command: run,
	    replaceDocument(document, historyPolicy = 'reset') {
	      closeMentions()
	      flush('replace')
	      suppressUpdate = true
	      documentMetadata = document.metadata ?? {}
	      const replacement = editor.schema.nodeFromJSON(v2ToPm(document))
	      if (historyPolicy === 'reset') {
	        editor.view.updateState(EditorState.create({
	          schema: editor.schema,
	          doc: replacement,
	          plugins: editor.state.plugins,
	        }))
	      } else {
	        const transaction = editor.state.tr
	          .replaceWith(0, editor.state.doc.content.size, replacement.content)
	          .setMeta('addToHistory', false)
	          .setMeta('externalReplacement', true)
	        editor.view.dispatch(transaction)
	      }
	      lastEmittedSnapshot = JSON.stringify(pmToV2(editor.getJSON(), documentMetadata))
	      suppressUpdate = false
	      updateSurfaces(editor)
	      emitCommandState(editor)
	    },
    snapshot: () => pmToV2(editor.getJSON(), documentMetadata),
	    destroy() {
	      if (destroyed) return
	      flush('destroy')
	      destroyed = true
	      closeMentions()
	      window.document.removeEventListener('pointerdown', outside, true)
	      if (geometryFrame !== null) window.cancelAnimationFrame(geometryFrame)
	      window.removeEventListener('scroll', refreshGeometry, true)
	      window.removeEventListener('resize', refreshGeometry)
	      window.visualViewport?.removeEventListener('resize', refreshGeometry)
	      window.visualViewport?.removeEventListener('scroll', refreshGeometry)
	      geometryObserver?.disconnect()
	      editor.view.dom.removeEventListener('pointermove', blockPointerMove)
	      editor.view.dom.removeEventListener('pointerleave', blockPointerLeave)
	      editor.view.dom.removeEventListener('keydown', editorKeydown, true)
      editor.destroy()
      bubble.remove(); blockControls.remove(); slash.remove(); blockMenu.remove(); linkPopover.remove(); tableControls.remove(); mediaPopover.remove(); mentions.remove()
    },
  }
}

export const createRegistry = () => {
  const sessions = new Map<string, EditorSession>()
  return {
    mount(id: string, host: HTMLElement, options: MountOptions) {
      sessions.get(id)?.destroy()
      const session = mount(host, options)
      sessions.set(id, session)
      return session
    },
    command(id: string, command: string, attrs?: Attributes) { return sessions.get(id)?.command(command, attrs) ?? false },
	    replaceDocument(id: string, document: ComponentDocumentV2, historyPolicy: 'preserve' | 'reset' = 'reset') {
	      sessions.get(id)?.replaceDocument(document, historyPolicy)
	    },
    snapshot(id: string) { return sessions.get(id)?.snapshot() },
    destroy(id: string) { sessions.get(id)?.destroy(); sessions.delete(id) },
  }
}

declare global { interface Window { __semanticDxEditor?: ReturnType<typeof createRegistry> } }

if (typeof window !== 'undefined') window.__semanticDxEditor ??= createRegistry()
