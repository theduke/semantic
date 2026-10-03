// One-off verification artifact: isolated fixture ids and hard-coded local ports.
// nix develop -c node docs/plans/2026-10-03-graph-views/browser.cjs
// Only writes its generated fixture to the isolated test server at port 8888.
const { chromium } = require('../../../crates/dxeditor/web/node_modules/playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const out = require('node:path').resolve(__dirname, '../../../target/graph-views-browser/review4');
fs.mkdirSync(out, { recursive: true });
const str = string => ({ string });
const obj = object => ({ object });
function decode(value) {
  if (!value || typeof value !== 'object') return value;
  if ('object' in value) return Object.fromEntries(Object.entries(value.object).map(([key, item]) => [key, decode(item)]));
  if ('list' in value) return value.list.map(decode);
  return Object.values(value)[0];
}
async function rpc(name, payload) {
  const response = await fetch(`http://127.0.0.1:8888/api/v1/rpc/${name}`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(obj(payload)) });
  const body = await response.json();
  assert(response.ok, JSON.stringify(body));
  return decode(body);
}
async function insert(id, title, parent) {
  const row = { id: str(id), type: str('semantic:base:label'), 'semantic:base:label:name': str(title), 'semantic:title': str(title) };
  if (parent) row['semantic:parent'] = str(parent);
  await rpc('semantic.db.insert', { id: str(id), object: obj(row) });
}
async function relation(id, source, target) {
  await rpc('semantic.db.insert', { id: str(id), object: obj({ id: str(id), type: str('semantic:base:entity_label'), 'semantic:relation:relation': str('semantic:base:entity_label'), 'semantic:relation:from': str(source), 'semantic:relation:to': str(target), 'semantic:base:entity_label:collection': str('entities') }) });
}
function node(page, id) { return page.locator(`[data-dxgraph-node="entity:8:entities${id}"]`); }
async function world(page) { return page.locator('.dxgraph-world').getAttribute('style'); }
async function assertNoOverlap(page) {
  const boxes = await page.locator('[data-dxgraph-node]').evaluateAll(nodes => nodes.filter(node => getComputedStyle(node).visibility !== 'hidden').map(node => { const r = node.getBoundingClientRect(); return { id: node.dataset.dxgraphNode, x: r.x, y: r.y, width: r.width, height: r.height }; }));
  for (let i = 0; i < boxes.length; i++) for (let j = i + 1; j < boxes.length; j++) {
    const a = boxes[i], b = boxes[j];
    assert(!(a.x < b.x + b.width - 1 && a.x + a.width > b.x + 1 && a.y < b.y + b.height - 1 && a.y + a.height > b.y + 1), `overlap: ${a.id} and ${b.id}`);
  }
  return boxes;
}
async function drag(page, from, delta) {
  await page.mouse.move(from.x, from.y); await page.mouse.down();
  await page.mouse.move(from.x + delta.x, from.y + delta.y, { steps: 12 }); await page.mouse.up();
}
(async () => {
  const prefix = `graph-browser-${Date.now()}`;
  const root = `${prefix}-root`, ancestor = `${prefix}-ancestor`, children = [], leaves = [];
  await insert(ancestor, 'Graph ancestor'); await insert(root, 'Graph browser root', ancestor);
  for (let i = 0; i < 4; i++) {
    const child = `${prefix}-child-${i}`; children.push(child); await insert(child, `Branch ${i + 1}`, root);
    for (let j = 0; j < 2 + i % 2; j++) { const leaf = `${prefix}-leaf-${i}-${j}`; leaves.push(leaf); await insert(leaf, `Leaf ${i + 1}.${j + 1}`, child); }
  }
  const label = `${prefix}-relation-target`; await insert(label, 'Related label');
  await relation(`${prefix}-rel-out`, root, label); await relation(`${prefix}-rel-in`, children[0], root);
  const missing = `${prefix}-missing`;
  // Missing rows are injected below: typed database references reject dangling inserts.
  const fixture = { root, ancestor, children, leaves, label, missing, url: `http://localhost:8080/graph?root=${root}` };
  fs.writeFileSync(`${out}/fixture.json`, JSON.stringify(fixture, null, 2));
  console.log('Fixture ready', fixture.url);
  const browser = await chromium.launch({ headless: true, executablePath: '/etc/profiles/per-user/theduke/bin/chromium', args: ['--no-sandbox'] });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  page.setDefaultTimeout(60000);
  let missingRowsInjected = 0;
  const pickerQueries = [];
  await page.route('**/api/v1/rpc', async route => {
    const request = route.request().postDataJSON();
    const payload = decode(request.payload);
    const query = JSON.stringify(payload.query);
    if (request.command === 'semantic.db.query' && typeof payload.query === 'object'
        && query.includes('semantic:title') && query.includes(root)) {
      pickerQueries.push(payload.query);
    }
    if (request.command !== 'semantic.db.query'
        || !JSON.stringify(payload.query).includes('__semantic.relationship_edges')
        || !payload.params?.ids?.includes(root)) {
      return route.continue();
    }
    const response = await route.fetch();
    const body = await response.json();
    const rows = body.result?.ok?.object?.rows?.list;
    assert(Array.isArray(rows), JSON.stringify(body));
    rows.push(obj({relation: str('semantic:base:entity_label'), source: str(root), target: str(missing)}));
    missingRowsInjected++;
    await route.fulfill({response, json: body});
  });
  const errors = []; page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
  const result = { fixture, checks: {}, errors };
  const assertEntityOnlyUrl = () => assert.equal(new URL(page.url()).searchParams.has('collection'), false);
  try {
    for (let attempt = 0; attempt < 30; attempt++) { await page.goto('http://localhost:8080/graph?collection=legacy-custom'); try { await page.getByRole('combobox', {name:'Graph root'}).waitFor({state:'visible',timeout:3000}); break; } catch { await page.waitForTimeout(1000); } }
    await page.getByRole('combobox', {name:'Graph root'}).fill(root);
    await page.getByRole('option').filter({hasText:root}).click(); await page.waitForURL(new RegExp(`root=${root}`)); result.checks.emptyRoutePickerSearch = true; console.log('Passed empty-route picker search');
    assertEntityOnlyUrl(); result.checks.legacyCollectionIgnoredByPicker = true;
    assert(pickerQueries.length, 'root picker query was not observed');
    for (const query of pickerQueries) assert(JSON.stringify(query).includes('"collection":"entities"'), JSON.stringify(query));
    result.checks.rootPickerQueriesDefaultEntities = true;
    await page.locator('.semantic-graph-list summary').waitFor({state:'visible'}); await node(page, children[0]).waitFor({ state: 'visible' });
    await page.locator('.dxgraph-node[style*="hidden"]').waitFor({ state: 'detached' });
    result.checks.initialNodes = await page.locator('[data-dxgraph-node]').count(); assert.equal(result.checks.initialNodes, 5);
    result.checks.initialBounds = await assertNoOverlap(page);
    await page.screenshot({ path: `${out}/hierarchy.png`, fullPage: true });
    const canvas = page.locator('.dxgraph'); const canvasBox = await canvas.boundingBox();
    for (const box of result.checks.initialBounds) assert(box.x >= canvasBox.x && box.y >= canvasBox.y && box.x + box.width <= canvasBox.x + canvasBox.width + 1 && box.y + box.height <= canvasBox.y + canvasBox.height + 1, `initial node outside canvas: ${box.id}`);
    result.checks.initialNeighborhoodFits = true;
    const positions = () => page.locator('[data-dxgraph-node]').evaluateAll(nodes => Object.fromEntries(nodes.map(node=>[node.dataset.dxgraphNode,node.style.transform])));
    const viewportState = async () => {
      const style = await world(page);
      const values = style.match(/translate\(([-.\d]+)px,\s*([-.\d]+)px\)\s*scale\(([-.\d]+)\)/);
      assert(values,style); return {x:Number(values[1]),y:Number(values[2]),zoom:Number(values[3])};
    };
    const lodPositions = await positions();
    await page.mouse.move(canvasBox.x+canvasBox.width/2,canvasBox.y+canvasBox.height/2);
    for(let step=0;step<10 && (await viewportState()).zoom>0.2;step++) {
      await page.mouse.wheel(0,250); await page.waitForTimeout(100);
      assert.deepEqual(await positions(),lodPositions,'LOD zoom-out moved nodes');
    }
    assert((await viewportState()).zoom<0.25,'did not cross Minimal LOD threshold');
    for(let step=0;step<10 && (await viewportState()).zoom<0.9;step++) {
      await page.mouse.wheel(0,-250); await page.waitForTimeout(100);
      assert.deepEqual(await positions(),lodPositions,'LOD zoom-in moved nodes');
    }
    result.checks.lodZoomKeepsNodePositions = true;
    await page.getByRole('button',{name:'Fit',exact:true}).click(); await page.waitForTimeout(100);
    // Change the canvas client origin without resizing it or clicking inside it.
    await page.locator('.semantic-graph-page').evaluate(element=>{element.style.marginTop='80px';});
    await page.waitForTimeout(100);
    const shiftedBox = await canvas.boundingBox();
    const local = {x:shiftedBox.width/2,y:shiftedBox.height/2};
    const oldView = await viewportState();
    const anchoredWorld = {x:(local.x-oldView.x)/oldView.zoom,y:(local.y-oldView.y)/oldView.zoom};
    await page.mouse.move(shiftedBox.x+local.x,shiftedBox.y+local.y); await page.mouse.wheel(0,120); await page.waitForTimeout(100);
    const newView = await viewportState();
    assert(Math.abs((local.x-newView.x)/newView.zoom-anchoredWorld.x)<1,'shifted wheel x anchor');
    assert(Math.abs((local.y-newView.y)/newView.zoom-anchoredWorld.y)<1,'shifted wheel y anchor');
    result.checks.shiftedContainerWheelAnchor = true;
    const burstZoom = (await viewportState()).zoom;
    for (let step = 0; step < 8; step++) await page.mouse.wheel(0, 20);
    await page.waitForTimeout(100);
    assert((await viewportState()).zoom < burstZoom, 'wheel burst lost zoom updates');
    result.checks.wheelBurst = true;
    await page.locator('.semantic-graph-page').evaluate(element=>{element.style.marginTop='';});
    await page.getByRole('button',{name:'Fit',exact:true}).click(); await page.waitForTimeout(100);
    let before = await world(page); await page.mouse.move(canvasBox.x + canvasBox.width / 2, canvasBox.y + canvasBox.height / 2); await page.mouse.wheel(0, 120);
    await page.waitForTimeout(100); assert.notEqual(await world(page), before); result.checks.wheelZoom = true; console.log('Passed wheel zoom');
    before = await world(page); await drag(page, { x: canvasBox.x + 24, y: canvasBox.y + 24 }, { x: 90, y: 50 });
    assert.notEqual(await world(page), before); result.checks.backgroundPan = true; console.log('Passed background pan');
    await page.getByRole('button', { name: 'Fit', exact: true }).click();
    const rootBox = await node(page, root).boundingBox(); const paths = await page.locator('.dxgraph-edge > path:first-child').evaluateAll(paths => paths.map(path => path.getAttribute('d')));
    const rootStyle = await node(page, root).getAttribute('style');
    await drag(page, { x: rootBox.x + 45, y: rootBox.y + 25 }, { x: 70, y: 35 });
    assert.notEqual(await node(page, root).getAttribute('style'), rootStyle);
    assert.notDeepEqual(await page.locator('.dxgraph-edge > path:first-child').evaluateAll(paths => paths.map(path => path.getAttribute('d'))), paths); result.checks.nodeDragUpdatesEdges = true; console.log('Passed node drag and edge updates');
    const draggedTransform = (await positions())[`entity:8:entities${root}`];
    await page.getByRole('button', {name:'Re-layout',exact:true}).click(); await page.waitForTimeout(100);
    assert.equal((await positions())[`entity:8:entities${root}`], draggedTransform, 'Re-layout should retain canvas drag positions');
    await page.getByRole('button', {name:'Reset positions',exact:true}).click();
    await page.waitForFunction(({id,transform})=>document.querySelector(`[data-dxgraph-node="${id}"]`)?.style.transform===transform, {id:`entity:8:entities${root}`,transform:lodPositions[`entity:8:entities${root}`]});
    const resetPositions = await positions();
    await page.getByRole('button', {name:'Re-layout',exact:true}).click(); await page.waitForTimeout(100);
    assert.deepEqual(await positions(), resetPositions, 'Reset positions did not restore automatic layout');
    result.checks.resetPositionsClearsCanvasDragPositions = true;
    await page.getByRole('button', {name:'Fit',exact:true}).click();
    await node(page, root).locator('strong').click(); await page.getByRole('complementary', { name: 'Selected entity' }).waitFor();
    assert(await page.locator('.semantic-graph-detail .semantic-entity-card').count()); result.checks.entityCard = true; console.log('Passed EntityCard panel');
    const panelBox = await page.getByRole('complementary', {name:'Selected entity'}).boundingBox();
    const protectedView = await world(page);
    await page.mouse.move(panelBox.x + 40, panelBox.y + 50); await page.mouse.wheel(0, 120); await page.waitForTimeout(100);
    assert.equal(await world(page), protectedView, 'detail panel wheel changed graph viewport');
    await drag(page, {x:panelBox.x + 40,y:panelBox.y + 50}, {x:20,y:15});
    assert.equal(await world(page), protectedView, 'detail panel drag changed graph viewport');
    result.checks.detailPanelStopsGraphGestures = true;
    await page.getByRole('button', { name: 'Load parent', exact: true }).click(); await node(page, ancestor).waitFor(); result.checks.loadParent = true;
    await page.getByRole('button', { name: 'Close', exact: true }).click();
    await node(page, children[0]).getByRole('button', { name: 'Expand node' }).click(); await node(page, leaves[0]).waitFor(); result.checks.expansion = true; console.log('Passed expansion');
    await page.getByRole('button', { name: 'Re-layout', exact: true }).click(); await page.getByRole('button', { name: 'Fit', exact: true }).click();
    result.checks.expandedBounds = await assertNoOverlap(page);
    const treePosition = await node(page, children[0]).getAttribute('style');
    await page.locator('.dx-select-trigger').click(); await page.getByRole('option', { name: 'Radial', exact: true }).click(); await page.waitForURL(/layout=radial/);
    assertEntityOnlyUrl(); result.checks.layoutUrlEntityOnly = true;
    await page.waitForFunction(({ id, style }) => document.querySelector(`[data-dxgraph-node=\"${id}\"]`)?.getAttribute('style') !== style, { id: `entity:8:entities${children[0]}`, style: treePosition });
    await page.keyboard.press('Escape'); await page.getByRole('button', { name: 'Fit', exact: true }).click(); await page.waitForTimeout(100);
    result.checks.radialLayout = true; await page.screenshot({ path: `${out}/radial.png`, fullPage: true });
    await page.getByRole('button', { name: 'Relations', exact: true }).click(); await page.waitForURL(/mode=relations/); await node(page, label).waitFor(); await node(page, children[0]).waitFor(); await node(page, missing).waitFor();
    assertEntityOnlyUrl(); result.checks.modeUrlEntityOnly = true;
    assert.equal(await node(page, missing).locator('[data-entity-kind="unresolved"]').count(), 1);
    assert.equal(await node(page, children[0]).locator('[data-entity-kind="entity"]').count(), 1);
    result.checks.defaultEntityIncomingAndMissing = true;
    console.log('Relation viewport', await viewportState());
    assert(await page.locator('.dxgraph-edge-label').count()); result.checks.relations = true;
    await page.waitForFunction(() => document.querySelector('.dx-select-trigger')?.textContent.includes('Force')); result.checks.modeLayoutControlSync = true;
    await page.getByRole('button', { name: 'Fit', exact: true }).click(); result.checks.relationBounds = await assertNoOverlap(page);
    await page.screenshot({ path: `${out}/relations.png`, fullPage: true });
    const summary = page.locator('.semantic-graph-list summary'); await summary.focus(); await page.keyboard.press('Enter');
    const list = page.getByRole('list', { name: 'Loaded graph entities' }); await list.waitFor({ state: 'visible' });
    assert(await list.getByRole('link', { name: 'Open entity' }).count() >= 2); result.checks.keyboardListFallback = true;
    const outgoingRow = list.getByRole('listitem').filter({hasText:'Related label'});
    await outgoingRow.getByRole('button', {name:'Related label',exact:true}).click();
    const outgoingPanel = page.getByRole('complementary', {name:'Selected entity'});
    assert(await outgoingPanel.locator('.semantic-entity-card').count());
    await outgoingPanel.getByRole('button', {name:'Expand',exact:true}).click();
    await outgoingPanel.getByRole('button', {name:'Collapse',exact:true}).waitFor();
    result.checks.outgoingTargetHasObjectAndExpands = true;
    await outgoingPanel.getByRole('button', {name:'Open',exact:true}).click();
    await page.waitForURL(url=>url.pathname!=='/graph'); result.checks.outgoingTargetOpens = true;
    await page.goto(`${fixture.url}&mode=relations`); await node(page,label).waitFor();
    await page.locator('.semantic-graph-list summary').click();
    await page.getByRole('list', {name:'Loaded graph entities'}).getByRole('listitem').filter({hasText:'Related label'}).getByRole('button', {name:'Related label',exact:true}).click();
    await page.getByRole('button', {name:'Focus here',exact:true}).click(); await page.waitForURL(new RegExp(`root=${label}`));
    assertEntityOnlyUrl(); result.checks.focusUrlEntityOnly = true;
    await node(page,label).waitFor(); result.checks.outgoingTargetFocuses = true;
    await page.goto(`${fixture.url}&mode=relations`); await node(page,label).waitFor();
    await page.locator('.semantic-graph-list summary').click();
    await summary.click(); await node(page, root).locator('strong').click();
    await page.getByRole('button', { name: 'Focus here', exact: true }).click();
    await node(page, root).waitFor({ state: 'visible' }); result.checks.focusHere = true;
    await node(page, missing).click();
    const unresolvedPanel = page.getByRole('complementary', { name: 'Selected entity' });
    await unresolvedPanel.getByText(`Entity unavailable: ${missing}`).waitFor();
    assert.equal(await unresolvedPanel.locator('.semantic-entity-card').count(), 0);
    result.checks.missingEntityHasStableIdentity = true;
    await page.getByRole('button', {name:'Close',exact:true}).click();
    await node(page, children[0]).click(); await page.getByRole('button', { name: 'Focus here', exact: true }).click();
    await page.waitForURL(new RegExp(`root=${children[0]}`)); await node(page, children[0]).waitFor({ state: 'visible' });
    const focused = await node(page, children[0]).boundingBox(), viewport = await canvas.boundingBox();
    assert(focused.x >= viewport.x && focused.x < viewport.x + viewport.width); result.checks.rerootFits = true;
    const currentCanvas = await canvas.boundingBox(); await drag(page, {x:currentCanvas.x + 24,y:currentCanvas.y + 24}, {x:450,y:80});
    const rootPicker = page.getByRole('combobox', {name:'Graph root'}); await rootPicker.fill('Graph browser root');
    await page.getByRole('option').filter({hasText:root}).click(); await page.waitForURL(new RegExp(`root=${root}`));
    assertEntityOnlyUrl(); result.checks.pickerUrlEntityOnly = true;
    await page.waitForFunction(() => document.querySelector('.dx-combobox-input')?.value === 'Graph browser root');
    await node(page, root).waitFor({state:'visible'}); result.checks.rootPicker = true;
    await page.setViewportSize({ width: 390, height: 844 }); await page.goto('http://localhost:8080/graph'); await page.getByText('Choose an entity above to explore its hierarchy and relationships.').waitFor();
    result.checks.emptyState = true;
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1), false); result.checks.mobileNoOverflow = true;
    await page.screenshot({ path: `${out}/mobile-empty.png`, fullPage: true });
    await page.getByRole('combobox', {name:'Graph root'}).fill(root); await page.getByRole('option').filter({hasText:root}).click();
    await node(page, children[0]).waitFor({state:'visible'});
    await page.keyboard.press('Escape'); await page.waitForTimeout(100);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1), false); result.checks.mobileGraph = true;
    await page.screenshot({path:`${out}/mobile-graph.png`,fullPage:true});
    assert(missingRowsInjected > 0, "missing endpoint response fixture was not exercised");
    result.checks.missingRowsInjected = missingRowsInjected;
    assert.deepEqual(errors, []);
    console.log(JSON.stringify(result.checks));
  } finally {
    fs.writeFileSync(`${out}/result.json`, JSON.stringify(result, null, 2));
    await page.screenshot({ path: `${out}/last.png`, fullPage: true }); await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
