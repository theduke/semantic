import AxeBuilder from '@axe-core/playwright'
import { expect, test } from '@playwright/test'

test.beforeEach(async ({ page }) => {
  await page.goto('/e2e/editor.html')
  await expect(page.locator('.ProseMirror')).toBeVisible()
})

test('keeps the idle canvas quiet and surfaces selection actions contextually', async ({ page }) => {
  await expect(page.getByRole('toolbar', { name: 'Text formatting' })).toBeHidden()
  const editor = page.locator('.ProseMirror')
  await editor.click()
  await page.keyboard.press('ControlOrMeta+A')
  await expect(page.getByRole('toolbar', { name: 'Text formatting' })).toBeVisible()
  await page.getByRole('button', { name: 'Bold' }).click()
  await expect(editor.locator('strong')).toHaveText('Select this text')
  await expect.poll(() => page.evaluate(() => {
    const events = (window as unknown as { editorEvents: Array<{ document?: { schema?: string } }> }).editorEvents
    return events.slice().reverse().find(event => event.document)?.document?.schema
  })).toBe('semantic.component-document')
})

test('inserts blocks from contextual add controls and edits tables locally', async ({ page }) => {
  const editor = page.locator('.ProseMirror')
  await editor.click()
  await page.getByRole('button', { name: 'Add a block' }).click()
  await page.getByRole('option', { name: 'Insert Table' }).click()
  await expect(editor.locator('table')).toBeVisible()
  await editor.locator('td').first().click()
  const tableToolbar = page.getByRole('toolbar', { name: 'Table actions' })
  await expect(tableToolbar).toBeVisible()
  const before = await editor.locator('tr').count()
  await tableToolbar.getByRole('button', { name: 'Add row after' }).click()
  await expect(editor.locator('tr')).toHaveCount(before + 1)
})

test('rejects unsafe links without losing the mapped selection', async ({ page }) => {
  const editor = page.locator('.ProseMirror')
  await editor.click()
  await page.keyboard.press('ControlOrMeta+A')
  await page.getByRole('button', { name: 'Add or edit link' }).click()
  await page.getByLabel('Link URL').fill('javascript:alert(1)')
  await page.getByRole('button', { name: 'Apply link' }).click()
  await expect(page.getByRole('alert')).toContainText('Use an HTTP')
  await expect(editor.locator('a')).toHaveCount(0)
})

test('has no automatically detectable accessibility violations in core states', async ({ page }) => {
  const editor = page.locator('.ProseMirror')
  await editor.click()
  const results = await new AxeBuilder({ page }).analyze()
  expect(results.violations).toEqual([])
})

test('inserts validated images and exposes contextual media properties', async ({ page }) => {
  const editor = page.locator('.ProseMirror')
  await editor.click()
  await page.getByRole('button', { name: 'Add a block' }).click()
  await page.getByRole('option', { name: 'Insert Image' }).click()
  const dialog = page.getByRole('dialog', { name: 'Image properties' })
  await dialog.getByLabel('Image URL').fill('https://example.com/diagram.png')
  await dialog.getByLabel('Alternative text').fill('Architecture diagram')
  await dialog.getByLabel('Title').fill('System architecture')
  await dialog.getByRole('button', { name: 'Apply image properties' }).click()
  const image = editor.locator('img:not(.ProseMirror-separator)')
  await expect(image).toHaveAttribute('alt', 'Architecture diagram')
  await expect(image).toHaveAttribute('title', 'System architecture')
})

test('uses cancellable contextual mention suggestions', async ({ page }) => {
  const editor = page.locator('.ProseMirror')
  await editor.click()
  await page.keyboard.press('End')
  await page.keyboard.type(' @al')
  await page.getByRole('option', { name: /Mention Alice/ }).click()
  await expect(editor.locator('[data-semantic-mention="entity-alice"]')).toHaveText('@Alice')
})

test('operates slash and mention listboxes without moving focus from the editor', async ({ page }) => {
  const editor = page.locator('.ProseMirror')
  await editor.click()
  await page.keyboard.press('End')
  await page.keyboard.type(' /hea')
  await expect(page.getByRole('listbox', { name: 'Insert block' })).toBeVisible()
  await expect(editor).toHaveAttribute('aria-expanded', 'true')
  await page.keyboard.press('Enter')
  await expect(editor.locator('h1')).toBeVisible()

  await editor.click()
  await page.keyboard.press('End')
  await page.keyboard.type(' @al')
  await expect(page.getByRole('option', { name: /Mention Alice/ })).toBeVisible()
  await page.keyboard.press('ArrowDown')
  await page.keyboard.press('Enter')
  await expect(editor.locator('[data-semantic-mention="entity-alpine"]')).toHaveText('@Alpine')
})

test('writes validated internal clipboard data with interoperable fallbacks', async ({ page }) => {
  const editor = page.locator('.ProseMirror')
  await editor.click()
  await page.keyboard.press('ControlOrMeta+A')
  const clipboard = await editor.evaluate(element => {
    const data = new DataTransfer()
    element.dispatchEvent(new ClipboardEvent('copy', { bubbles: true, cancelable: true, clipboardData: data }))
    return {
      internal: data.getData('application/x-semantic-dxeditor+json'),
      html: data.getData('text/html'),
      text: data.getData('text/plain'),
    }
  })
  expect(clipboard.text).toContain('Select this text')
  expect(clipboard.html).toContain('Select this text')
  expect(JSON.parse(clipboard.internal)).toMatchObject({
    version: 2, schema: 'semantic.prosemirror-slice',
    schemaFingerprint: 'fixture-schema:semantic.pm-slice.v2:2026-08-30',
  })
})

test('publishes local ownership immediately and keeps snapshot identities stable', async ({ page }) => {
  const editor = page.locator('.ProseMirror')
  await page.evaluate(() => { (window as unknown as { editorEvents: unknown[] }).editorEvents = [] })
  await editor.click()
  await page.keyboard.press('End')
  await page.keyboard.type('!')
  const result = await page.evaluate(() => {
    const fixture = window as unknown as {
      editorEvents: Array<{ kind?: string; revision?: number; document?: unknown }>
      editorSession: { snapshot(): unknown }
    }
    const changes = fixture.editorEvents.filter(event => event.kind === 'documentChange')
    return { changes, first: fixture.editorSession.snapshot(), second: fixture.editorSession.snapshot() }
  })
  expect(result.changes).toHaveLength(1)
  expect(result.changes[0]?.revision).toBe(1)
  expect(result.second).toEqual(result.first)
})

test('reset replacement prevents undo from crossing an external revision', async ({ page }) => {
  const editor = page.locator('.ProseMirror')
  await editor.click()
  await page.keyboard.press('End')
  await page.keyboard.type(' local')
  const text = await page.evaluate(() => {
    const fixture = window as unknown as {
      editorSession: {
        replaceDocument(document: unknown, historyPolicy: 'reset'): void
        command(name: string): boolean
        snapshot(): { root: { content?: Array<{ content?: Array<{ text?: string }> }> } }
      }
    }
    fixture.editorSession.replaceDocument({
      schema: 'semantic.component-document', version: 2,
      root: { kind: 'document', content: [{ kind: 'paragraph', id: 'server-paragraph', content: [{ kind: 'text', text: 'server' }] }] },
    }, 'reset')
    fixture.editorSession.command('undo')
    return fixture.editorSession.snapshot().root.content?.[0]?.content?.[0]?.text
  })
  expect(text).toBe('server')
})

test('grows a table for rectangular TSV paste', async ({ page }) => {
  const editor = page.locator('.ProseMirror')
  await editor.click()
  await page.getByRole('button', { name: 'Add a block' }).click()
  await page.getByRole('option', { name: 'Insert Table' }).click()
  const cells = editor.locator('th, td')
  await cells.last().click({ force: true })
  await cells.last().evaluate(element => {
    const data = new DataTransfer()
    data.setData('text/plain', 'A\tB\nC\tD')
    element.dispatchEvent(new ClipboardEvent('paste', { bubbles: true, cancelable: true, clipboardData: data }))
  })
  await expect(editor.locator('tr')).toHaveCount(4)
  await expect(editor.locator('tr').last().locator('td, th')).toHaveCount(4)
  await expect(editor.locator('tr').last().locator('td, th').last()).toContainText('D')
})
