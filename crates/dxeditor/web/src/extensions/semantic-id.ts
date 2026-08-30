import { Extension } from '@tiptap/core'
import { Plugin } from '@tiptap/pm/state'

export const IDENTITY_NODE_TYPES = new Set([
  'paragraph', 'heading', 'blockquote', 'codeBlock', 'bulletList', 'orderedList',
  'listItem', 'taskList', 'taskItem', 'hardBreak', 'horizontalRule', 'table', 'tableRow',
  'tableCell', 'tableHeader', 'image', 'semanticMention', 'opaqueBlock', 'opaqueInline',
])

let fallbackSequence = 0

export const generateSemanticId = (): string => {
  if (typeof globalThis.crypto?.randomUUID === 'function') return globalThis.crypto.randomUUID()
  fallbackSequence += 1
  return `semantic-${Date.now().toString(36)}-${fallbackSequence.toString(36)}`
}

export const SemanticId = Extension.create({
  name: 'semanticId',

  addGlobalAttributes() {
    return [{
      types: [...IDENTITY_NODE_TYPES],
      attributes: {
        semanticId: {
          default: null,
          parseHTML: element => element.getAttribute('data-semantic-id'),
          renderHTML: attrs => attrs.semanticId ? { 'data-semantic-id': attrs.semanticId } : {},
        },
        alignment: {
          default: null,
          parseHTML: element => element.getAttribute('data-alignment'),
          renderHTML: attrs => ['left', 'center', 'right'].includes(String(attrs.alignment))
            ? { 'data-alignment': attrs.alignment, style: `text-align: ${attrs.alignment}` }
            : {},
        },
      },
    }]
  },

  addProseMirrorPlugins() {
    return [new Plugin({
      appendTransaction(transactions, _oldState, newState) {
        if (!transactions.some(transaction => transaction.docChanged)) return null
        const seen = new Set<string>()
        let transaction = newState.tr
        let changed = false
        newState.doc.descendants((node, position) => {
          if (!IDENTITY_NODE_TYPES.has(node.type.name)) return
          const current = typeof node.attrs.semanticId === 'string' ? node.attrs.semanticId.trim() : ''
          if (current && !seen.has(current)) {
            seen.add(current)
            return
          }
          let generated = generateSemanticId()
          while (seen.has(generated)) generated = generateSemanticId()
          seen.add(generated)
          transaction = transaction.setNodeMarkup(position, undefined, { ...node.attrs, semanticId: generated })
          changed = true
        })
        return changed
          ? transaction.setMeta('addToHistory', false).setMeta('semanticIdentityMaintenance', true)
          : null
      },
    })]
  },
})

