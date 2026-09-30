// Run through: nix develop .#ui -c node docs/plans/2026-09-30-task-system/fix-browser.cjs
// Requires isolated server at8888 and freshly rebuilt UI at8080. Creates its own fixture.
const { chromium } = require('../../../crates/dxeditor/web/node_modules/playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const out = '/tmp/semantic-task-browser-evidence/fixes';
fs.mkdirSync(out, { recursive: true });
const str = string => ({ string });
const obj = object => ({ object });
function decode(value) {
  if (typeof value !== 'object' || value === null) return value;
  if ('object' in value) return Object.fromEntries(Object.entries(value.object).map(([key, item]) => [key, decode(item)]));
  if ('list' in value) return value.list.map(decode);
  if ('variant' in value) return value.variant.variant;
  return Object.values(value)[0];
}
async function rpc(name, payload) {
  const response = await fetch(`http://127.0.0.1:8888/api/v1/rpc/${name}`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(obj(payload)) });
  const body = await response.json();
  assert(response.ok, JSON.stringify(body));
  return decode(body);
}
function content(body) {
  const now = { date_time: Date.now() * 1000000 };
  return obj({ kind: str('note'), data: obj({
    'semantic:base:note:note_format': str('markdown'), 'semantic:base:note:note_content': str(body),
    'semantic:created_at': now, 'semantic:updated_at': now,
  }) });
}
(async () => {
  const browser = await chromium.launch({ headless: true, executablePath: '/etc/profiles/per-user/theduke/bin/chromium', args: ['--no-sandbox'] });
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
  const a = await context.newPage();
  const b = await context.newPage();
  const errors = [];
  for (const page of [a, b]) {
    page.setDefaultTimeout(20000);
    page.on('pageerror', error => errors.push(error.message));
    page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
  }
  async function create(title) {
    await a.goto('http://localhost:8080/tasks/create');
    await a.getByLabel('Task title').fill(title);
    await a.getByRole('button', { name: 'Create task', exact: true }).click();
    await a.waitForURL(/\/tasks\/task-/);
    await a.getByRole('button', { name: 'Save changes', exact: true }).waitFor();
    return { url: a.url(), id: a.url().split('/').pop() };
  }
  async function open(page, task) { await page.goto(task.url); await page.getByLabel('Task title').waitFor(); }
  async function save(page) {
    await page.getByRole('button', { name: 'Save changes', exact: true }).click();
    await page.getByText('Changes saved', { exact: true }).waitFor();
    assert.equal(await page.getByText('Unsaved changes', { exact: true }).count(), 0);
    assert(await page.getByRole('button', { name: 'Save changes', exact: true }).isDisabled());
  }
  const parent = await create('Fix verification parent');
  const child = await create('Fix verification concurrent task');
  await open(b, child);
  await b.getByLabel('Priority', { exact: true }).selectOption('urgent');
  await b.getByLabel('Status', { exact: true }).selectOption('in_progress');
  await b.locator('input[type=number]').fill('40');
  await b.locator('input[type=date]').fill('2026-10-15');
  await b.getByLabel('Parent task', { exact: true }).selectOption(parent.id);
  await b.locator('.semantic-task-description [contenteditable=true]').fill('Remote description after two-client save.');
  await save(b);
  await a.getByLabel('Task title').fill('Local title after remote metadata');
  await save(a);
  assert.equal(await a.getByLabel('Priority', { exact: true }).inputValue(), 'urgent');
  assert.equal(await a.getByLabel('Status', { exact: true }).inputValue(), 'in_progress');
  assert.equal(await a.locator('input[type=number]').inputValue(), '40');
  assert.equal(await a.locator('input[type=date]').inputValue(), '2026-10-15');
  assert.equal(await a.getByLabel('Parent task', { exact: true }).inputValue(), parent.id);
  await a.locator('.semantic-task-description [contenteditable=true]').filter({ hasText: 'Remote description after two-client save.' }).waitFor();
  await a.getByLabel('Task title').fill('Second local title preserves remote fields');
  await save(a);
  await open(b, child);
  assert.equal(await b.getByLabel('Priority', { exact: true }).inputValue(), 'urgent');
  assert.equal(await b.locator('input[type=number]').inputValue(), '40');
  console.log('PASS TWO_TAB_SAVE_CLEAN_CONTENT_AND_REMOTE_METADATA_PRESERVED');

  await b.getByLabel('Priority', { exact: true }).selectOption('high');
  await b.locator('input[type=number]').fill('65');
  await b.locator('.semantic-task-description [contenteditable=true]').fill('Remote description after archive.');
  await save(b);
  await a.getByRole('button', { name: 'Archive task', exact: true }).click();
  await a.getByRole('button', { name: 'Restore task', exact: true }).waitFor();
  assert.equal(await a.getByText('Unsaved changes', { exact: true }).count(), 0);
  assert.equal(await a.getByLabel('Priority', { exact: true }).inputValue(), 'high');
  assert.equal(await a.locator('input[type=number]').inputValue(), '65');
  await a.locator('.semantic-task-description [contenteditable=true]').filter({ hasText: 'Remote description after archive.' }).waitFor();
  await a.getByLabel('Task title').fill('Archive response preserves remote fields');
  await save(a);
  await a.getByRole('button', { name: 'Restore task', exact: true }).click();
  await a.getByRole('button', { name: 'Archive task', exact: true }).waitFor();
  await open(b, child);
  assert.equal(await b.getByLabel('Priority', { exact: true }).inputValue(), 'high');
  assert.equal(await b.locator('input[type=number]').inputValue(), '65');
  console.log('PASS TWO_TAB_ARCHIVE_RECONCILIATION_AND_FOLLOWING_SAVE');

  await open(b, parent);
  await b.getByRole('button', { name: 'Archive task', exact: true }).click();
  await b.getByRole('button', { name: 'Restore task', exact: true }).waitFor();
  await open(a, child);
  const selector = a.getByLabel('Parent task', { exact: true });
  await selector.locator(`option[value="${parent.id}"]`).filter({ hasText: 'Fix verification parent (archived)' }).waitFor({ state: 'attached' });
  assert.equal(await selector.inputValue(), parent.id);
  assert.equal(await selector.locator('option:checked').innerText(), 'Fix verification parent (archived)');
  await selector.selectOption('');
  await selector.selectOption(parent.id);
  await a.getByLabel('Task title').fill('Archived parent relationship stays intact');
  await save(a);
  await open(a, child);
  await selector.locator(`option[value="${parent.id}"]`).filter({ hasText: 'Fix verification parent (archived)' }).waitFor({ state: 'attached' });
  assert.equal(await selector.inputValue(), parent.id);
  console.log('PASS ARCHIVED_PARENT_DISPLAY_RESELECT_AND_SAVE');

  const target = obj({ collection: str('entities'), id: str(child.id) });
  const seeds = [];
  console.log('Seeding 105 comments in isolated fixture');
  for (let index = 0; index < 105; index++) {
    seeds.push(await rpc('semantic.comments.create', { scope_id: 'null', target, main_content: content(`Existing chronological comment ${index}`), parent: 'null' }));
  }
  await open(a, child);
  await a.locator('.semantic-comment').nth(99).waitFor();
  assert.equal(await a.locator('.semantic-comment').count(), 100);
  await a.locator('section.semantic-comments > .semantic-comment-composer [contenteditable=true]').fill('Posted root beyond initial hundred');
  await a.getByRole('button', { name: 'Post comment', exact: true }).click();
  await a.locator('.semantic-comment .semantic-content-view').getByText('Posted root beyond initial hundred', { exact: true }).waitFor();
  assert.equal(await a.locator('.semantic-comment').count(), 106);
  assert.equal(await a.getByText('Posted root beyond initial hundred', { exact: true }).count(), 1);
  await a.reload();
  await a.locator('.semantic-comment').nth(99).waitFor();
  assert.equal(await a.locator('.semantic-comment').count(), 100);
  const visibleParent = a.locator('.semantic-comment').filter({ hasText: 'Existing chronological comment 0' }).first();
  await visibleParent.getByRole('button', { name: 'Reply', exact: true }).click();
  await visibleParent.locator('[contenteditable=true]').fill('Posted reply beyond initial hundred');
  await visibleParent.getByRole('button', { name: 'Post reply', exact: true }).click();
  await a.locator('.semantic-comment .semantic-content-view').getByText('Posted reply beyond initial hundred', { exact: true }).waitFor();
  assert.equal(await a.locator('.semantic-comment').count(), 107);
  assert.equal(await a.locator('.semantic-comment').nth(1).getByText('Posted reply beyond initial hundred', { exact: true }).count(), 1);
  await a.getByLabel('Threaded', { exact: true }).uncheck();
  assert.equal(await a.locator('.semantic-comment').last().getByText('Posted reply beyond initial hundred', { exact: true }).count(), 1);
  await a.getByLabel('Threaded', { exact: true }).check();
  const first = a.locator('.semantic-comment').first();
  await first.getByRole('button', { name: 'Delete', exact: true }).click();
  await a.getByText('This comment was deleted.', { exact: true }).waitFor();
  await a.locator('.semantic-comment .semantic-content-view').getByText('Posted reply beyond initial hundred', { exact: true }).waitFor();
  const listed = await rpc('semantic.comments.list', { scope_id: 'null', target, offset: { u64: 0 }, limit: { u64: 100 } });
  assert.equal(listed.comments[0].id, seeds[0].id);
  assert.equal(listed.comments[0].deleted, true);
  console.log('PASS ROOT_AND_VISIBLE_PARENT_REPLY_PAST100_FLAT_THREAD_ORDER_TOMBSTONE');

  await a.evaluate(() => scrollTo(0, 0));
  await a.screenshot({ path: `${out}/detail-desktop.png` });
  await a.locator('.semantic-comments__header').evaluate(el => { el.scrollIntoView({block:'start'}); window.scrollBy(0,-90); });
  await a.screenshot({ path: `${out}/comments-desktop.png` });
  await a.setViewportSize({ width: 390, height: 844 });
  await a.evaluate(() => scrollTo(0, 0));
  await a.screenshot({ path: `${out}/detail-mobile.png` });
  await a.locator('.semantic-comments__header').evaluate(el => { el.scrollIntoView({block:'start'}); window.scrollBy(0,-90); });
  await a.screenshot({ path: `${out}/comments-mobile.png` });
  assert.equal(await a.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
  console.log('PASS MOBILE_DETAIL_NO_HORIZONTAL_OVERFLOW');
  await a.setViewportSize({ width: 1440, height: 1000 });
  for (const view of ['cards', 'table']) {
    for (const classId of ['semantic:tasks:task', 'semantic:comments:comment']) {
      const sql = `SELECT * FROM entities WHERE type = '${classId}'`;
      await a.goto(`http://localhost:8080/browse?view=${view}&sql=${encodeURIComponent('inline:' + sql)}`);
      await a.locator(view === 'table' ? '.semantic-entity-list__actions-cell' : '.semantic-entity-card').first().waitFor();
      assert.equal(await a.getByRole('button', { name: 'Delete entity', exact: true }).count(), 0);
      assert(await a.getByRole('button', { name: 'Edit labels', exact: true }).count() > 0);
    }
  }
  await a.screenshot({ path: `${out}/generic-comment-table.png` });
  await a.goto(`http://localhost:8080/entities/${seeds[0].id}`);
  await a.getByText('This comment was deleted.', { exact: true }).waitFor();
  assert.equal(await a.getByRole('button', { name: 'Delete entity', exact: true }).count(), 0);
  await a.goto(`http://localhost:8080/entities/${child.id}`);
  await a.getByLabel('Task title').waitFor();
  assert.equal(await a.getByRole('button', { name: 'Delete entity', exact: true }).count(), 0);
  console.log('PASS GENERIC_CARD_TABLE_DETAIL_NO_HARD_DELETE');
  await a.goto('http://localhost:8080/tasks');
  await a.locator('.semantic-task-row').first().waitFor();
  await a.setViewportSize({ width: 390, height: 844 });
  await a.screenshot({ path: `${out}/workspace-mobile.png`, fullPage: true });
  assert.equal(await a.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
  assert.deepEqual(errors, []);
  fs.writeFileSync(`${out}/result.json`, JSON.stringify({ parent, child, comments: 107, errors }, null, 2));
  console.log('PASS BROWSER_ERRORS_EMPTY', JSON.stringify({ parent, child, errors }));
  await browser.close();
})().catch(error => { console.error(error); process.exit(1); });
