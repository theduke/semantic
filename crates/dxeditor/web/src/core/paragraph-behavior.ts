import { Extension } from '@tiptap/core'
import { Fragment, Slice, type Mark, type Schema } from '@tiptap/pm/model'

/** Build a paste slice where a newline is an inline break and a blank line is a block boundary. */
export const plainTextParagraphSlice = (
  text: string,
  schema: Schema,
  marks: readonly Mark[] = [],
): Slice => {
  const paragraphs = text.replace(/\r\n?/gu, '\n').split(/\n(?:[\t ]*\n)+/gu)
  const nodes = paragraphs.map(paragraph => {
    const content = paragraph.split('\n').flatMap((line, index) => [
      ...(index > 0 ? [schema.nodes.hardBreak.create()] : []),
      ...(line ? [schema.text(line, marks)] : []),
    ])
    return schema.nodes.paragraph.create(null, content)
  })
  return Slice.maxOpen(Fragment.from(nodes), true)
}

/**
 * Mandatory paragraph behavior installed by the editor engine. Tiptap's
 * extension primitive is only the internal keymap composition mechanism; this
 * behavior is not part of dxeditor's consumer extension surface.
 *
 * Ordinary newlines stay inside a top-level text block. A second Enter on the
 * resulting empty line removes the delimiter break and starts a new semantic
 * paragraph block. Nested paragraphs deliberately keep Tiptap's native Enter
 * behavior so list, task, table, and quote keyboard semantics remain intact.
 */
export const CoreParagraphBehavior = Extension.create({
  name: 'coreParagraphBehavior',
  priority: 1_000,

  addKeyboardShortcuts() {
    return {
      Enter: () => {
        const { selection } = this.editor.state
        const { $from, $to } = selection
        if (
          !$from.sameParent($to)
          || $from.depth !== 1
          || $from.parent.type.name !== 'paragraph'
        ) return false

        const previous = selection.empty ? $from.nodeBefore : null
        if (selection.empty && (!previous || previous.type.name === 'hardBreak')) {
          const chain = this.editor.chain()
          if (previous?.type.name === 'hardBreak') {
            chain.command(({ tr }) => {
              tr.delete($from.pos - previous.nodeSize, $from.pos)
              return true
            })
          }
          return chain.splitBlock().run()
        }

        return this.editor.commands.setHardBreak()
      },
    }
  },
})
