import { readFileSync } from 'node:fs'
import { expect, test, type Page } from '@playwright/test'
import type { ComponentDocumentV2 } from '../src/index'

type ComponentNodeV2 = ComponentDocumentV2['root']

// Exercise the real CSS cascade, including the application reset that removes
// browser-default list padding. The generic fixture styles do not reproduce it.
const componentSource = readFileSync(new URL('../../src/component.rs', import.meta.url), 'utf8')
const editorStyles = componentSource.match(/const DXEDITOR_STYLE: &str = r#"([\s\S]*?)"#;/)![1]
const applicationStyles = ['../../../dxcomp/assets/dxcomp.css', '../../../ui/assets/core_styles.css']
  .map(path => readFileSync(new URL(path, import.meta.url), 'utf8')).join('\n')

const replaceContent = async (page: Page, content: ComponentNodeV2[]) => {
  await page.evaluate(content => {
    const session = (window as unknown as {
      editorSession: { replaceDocument: (document: ComponentDocumentV2) => void }
    }).editorSession
    session.replaceDocument({
      schema: 'semantic.component-document', version: 2,
      root: { kind: 'document', content },
    })
  }, content)
}

const paragraph = (id: string, text = ''): ComponentNodeV2 => ({
  id, kind: 'paragraph', content: text ? [{ kind: 'text', text }] : [],
})

test.beforeEach(async ({ page }) => {
  await page.goto('/e2e/editor.html')
  await expect(page.locator('.ProseMirror')).toBeVisible()
  await page.evaluate(() => {
    document.querySelectorAll('style').forEach(style => style.remove())
    document.querySelector('h1')?.remove()
    const form = document.createElement('section')
    form.className = 'semantic-note-form'
    form.dataset.format = 'markdown'
    const field = document.createElement('div')
    field.className = 'semantic-note-form__document'
    field.append(document.querySelector('.dxeditor')!)
    form.append(field)
    document.body.append(form)
    document.querySelector('.dxeditor')!.setAttribute('data-engine', 'tiptap-prosemirror')
    document.querySelector('[data-dxeditor-host]')!.className = 'dxeditor__document'
    document.querySelector('[data-dxeditor-overlays]')!.className = 'dxeditor__overlays'
  })
  await page.addStyleTag({ content: applicationStyles + '\n' + editorStyles })
})

test('shows the writing hint only for a genuinely empty document', async ({ page }) => {
  const editor = page.locator('.ProseMirror')
  const hint = () => editor.locator('p').first().evaluate(p => getComputedStyle(p, '::before').content)
  await replaceContent(page, [paragraph('empty')])
  await expect.poll(hint).toContain('Write, or type /')
  await editor.click()
  await page.keyboard.type('First line')
  await expect.poll(hint).toBe('none')
  await page.keyboard.press('Shift+Enter')
  await expect(editor.locator('p > br.ProseMirror-trailingBreak')).toHaveCount(1)
  await expect.poll(hint).toBe('none')
  await page.keyboard.type('Second line')
  await expect.poll(hint).toBe('none')

  await page.keyboard.press('ControlOrMeta+A')
  await page.keyboard.press('Backspace')
  await expect.poll(hint).toContain('Write, or type /')
  await replaceContent(page, [{
    ...paragraph('loaded', 'Saved line'),
    content: [{ kind: 'text', text: 'Saved line' }, { kind: 'hard_break', id: 'break' }],
  }])
  await expect.poll(hint).toBe('none')
  await replaceContent(page, [{ kind: 'paragraph', id: 'break-only', content: [{ kind: 'hard_break', id: 'break' }] }])
  await expect.poll(hint).toBe('none')
  await replaceContent(page, [paragraph('empty-again')])
  await expect.poll(hint).toContain('Write, or type /')
})

for (const kind of ['bullet_list', 'ordered_list', 'task_list'] as const) {
  test(`indents each nested ${kind} level with application styles`, async ({ page }) => {
    const list = (depth: number): ComponentNodeV2 => ({
      kind, id: `list-${depth}`, content: [{
        kind: kind === 'task_list' ? 'task_item' : 'list_item', id: `item-${depth}`,
        content: [paragraph(`paragraph-${depth}`, `Level ${depth}`), ...(depth < 2 ? [list(depth + 1)] : [])],
      }],
    })
    await replaceContent(page, [list(0)])
    const editor = page.locator('.ProseMirror')
    const boxes = await Promise.all([0, 1, 2].map(depth =>
      editor.locator(`[data-semantic-id="paragraph-${depth}"]`).boundingBox()))
    for (let depth = 1; depth < boxes.length; depth += 1) {
      expect(boxes[depth]!.x - boxes[depth - 1]!.x).toBeGreaterThanOrEqual(16)
    }
    const marker = kind === 'ordered_list' ? 'decimal' : kind === 'task_list' ? 'none' : 'disc'
    await expect(editor.locator('ul, ol').first()).toHaveCSS('list-style-type', marker)
    if (kind === 'task_list') await expect(editor.locator('input[type="checkbox"]')).toHaveCount(3)
  })
}

for (const width of [1280, 390]) {
  test(`uses the available canvas without excess blank space at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 })
    await replaceContent(page, [paragraph('compact', 'A short note')])
    const host = await page.locator('[data-dxeditor-host]').boundingBox()
    const editor = await page.locator('.ProseMirror').boundingBox()
    expect(host!.height).toBeLessThanOrEqual(180)
    expect(editor!.x - host!.x).toBeLessThanOrEqual(36)
    expect(host!.width - editor!.width).toBeLessThanOrEqual(56)
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width)
  })
}
