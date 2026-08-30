import type { JSONContent } from '@tiptap/core'
import { generateSemanticId, IDENTITY_NODE_TYPES } from './extensions/semantic-id'

const remapEmbeddedDocumentNode = (value: unknown): unknown => {
  if (Array.isArray(value)) return value.map(remapEmbeddedDocumentNode)
  if (!value || typeof value !== 'object') return value
  const object = value as Record<string, unknown>
  return Object.fromEntries(Object.entries(object).map(([name, child]) => {
    if (name === 'id' && typeof child === 'string') return [name, generateSemanticId()]
    return [name, remapEmbeddedDocumentNode(child)]
  }))
}

export const stripCopiedIdentities = (node: JSONContent): JSONContent => {
  const attrs = node.attrs && IDENTITY_NODE_TYPES.has(node.type ?? '')
    ? Object.fromEntries(Object.entries(node.attrs)
      .filter(([name]) => name !== 'semanticId')
      .map(([name, value]) => [name, name === 'originalNode' ? remapEmbeddedDocumentNode(value) : value]))
    : node.attrs
  return {
    ...node,
    ...(attrs ? { attrs } : {}),
    ...(node.content ? { content: node.content.map(stripCopiedIdentities) } : {}),
  }
}

export const validateSemanticIds = (root: JSONContent): string | null => {
  const seen = new Set<string>()
  const visit = (node: JSONContent): string | null => {
    if (IDENTITY_NODE_TYPES.has(node.type ?? '')) {
      const id = typeof node.attrs?.semanticId === 'string' ? node.attrs.semanticId.trim() : ''
      if (!id) return `node '${node.type}' is missing semanticId`
      if (seen.has(id)) return `duplicate semanticId '${id}'`
      seen.add(id)
    }
    for (const child of node.content ?? []) {
      const issue = visit(child)
      if (issue) return issue
    }
    return null
  }
  return visit(root)
}
