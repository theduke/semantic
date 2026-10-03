// One-off verification artifact: hard-coded inspector ports and fixture assumptions.
// Test-only WebKitGTK inspector + real X11 input. No application JS is injected.
// INSPECTOR_PORT=9224 DISPLAY=:93 nix develop -c node docs/plans/2026-10-03-graph-views/desktop.mjs
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
const port = process.env.INSPECTOR_PORT || '9224';
const semantic = port === '9223';
const prefix = semantic ? 'semantic-graph' : 'demo-300';
const rootId = semantic ? 'entity:8:entitiesgraph-desktop-note-root' : 'node-000';
const rootQuery = `document.querySelector('[data-dxgraph-node="${rootId}"]')`;
const html = await (await fetch(`http://127.0.0.1:${port}/`)).text();
const socketPath = html.match(/\/socket\/\d+\/\d+\/WebPage/)[0];
const ws = new WebSocket(`ws://127.0.0.1:${port}${socketPath}`);
let target, next = 1;
const pending = new Map();
let readyResolve;
const ready = new Promise(resolve => readyResolve = resolve);
ws.onmessage = event => {
    const message = JSON.parse(event.data);
    if (message.method === 'Target.targetCreated' && message.params.targetInfo.type === 'page') {
        target = message.params.targetInfo.targetId; readyResolve();
    }
    if (message.method === 'Target.dispatchMessageFromTarget') {
        const inner = JSON.parse(message.params.message);
        const request = pending.get(inner.id);
        if (request) {
            pending.delete(inner.id); clearTimeout(request.timer);
            if (inner.error) request.reject(inner.error); else request.resolve(inner.result);
        }
    }
};
async function evaluate(expression) {
    await ready;
    const id = next++;
    return new Promise((resolve, reject) => {
        const timer = setTimeout(() => { pending.delete(id); reject(new Error('Inspector request timed out')); }, 5000);
        pending.set(id, { timer, reject, resolve: result => result.wasThrown ? reject(new Error(expression + ": " + result.result.description)) : resolve(result.result.value) });
        ws.send(JSON.stringify({ id: id + 100000, method: 'Target.sendMessageToTarget', params: {
            targetId: target, message: JSON.stringify({ id, method: 'Runtime.evaluate', params: { expression, returnByValue: true } }),
        } }));
    });
}
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
async function wait(expression, predicate, description) {
    const deadline = performance.now() + 10000;
    do { const value = await evaluate(expression); if (predicate(value)) return value; await sleep(5); } while (performance.now() < deadline);
    throw new Error(`Timed out waiting for ${description}`);
}
const output = resolve('target/graph-views-desktop'); mkdirSync(output, { recursive: true });
const pid = process.env.DESKTOP_PID || execFileSync('pgrep', ['-x', port === '9224' ? 'demo' : 'semantic_ui'], { encoding: 'utf8' }).trim().split('\n')[0];
const windows = execFileSync('xdotool', ['search', '--onlyvisible', '--pid', pid], { encoding: 'utf8' }).trim().split('\n');
const windowId = windows.map(id=>{const bounds=execFileSync('xdotool',['getwindowgeometry','--shell',id],{encoding:'utf8'});return {id,area:Number(bounds.match(/WIDTH=(\d+)/)[1])*Number(bounds.match(/HEIGHT=(\d+)/)[1])};}).sort((a,b)=>b.area-a.area)[0].id;
const input = (...args) => execFileSync('xdotool', args.map(String), { env: process.env });
input('windowraise', windowId, 'windowfocus', windowId); input('mouseup', 1);
const geometry = await evaluate('({dpr:devicePixelRatio,x:screenX,y:screenY,outerHeight,innerHeight})');
const point = (x, y) => [Math.round(geometry.x + x * geometry.dpr), Math.round(geometry.y + geometry.outerHeight - geometry.innerHeight * geometry.dpr + y * geometry.dpr)];
const move = (x, y) => input('mousemove', ...point(x, y));
const click = (x, y) => { move(x, y); input('click', 1); };
const transformExpr = 'document.querySelector(".dxgraph-world")?.style.transform';
if (semantic && !(await evaluate(rootQuery))) {
    const graphLink = await evaluate(`[...document.querySelectorAll('a')].find(e=>e.getAttribute('href')==='/graph?' && e.getBoundingClientRect().width>0)?.getBoundingClientRect().toJSON()`);
    if (graphLink) click(graphLink.x + graphLink.width/2, graphLink.y + graphLink.height/2);
    const picker = await wait(`document.querySelector('input[role=combobox]')?.getBoundingClientRect().toJSON()`, Boolean, 'root picker');
    click(picker.x + picker.width/2, picker.y + picker.height/2);
    input('key', 'ctrl+a'); input('type', '--delay', 90, 'Desktop graph note');
    const option = await wait(`[...document.querySelectorAll('[role=option]')].find(e=>e.textContent.includes('graph-desktop-note-root'))?.getBoundingClientRect().toJSON()`, Boolean, 'native fixture option');
    click(option.x + option.width/2, option.y + option.height/2);
}
const snapshotExpr = `({nodes:document.querySelectorAll('.dxgraph-node').length,hidden:document.querySelectorAll('.dxgraph-node[style*=hidden]').length,edges:document.querySelectorAll('.dxgraph-edge').length,transform:${transformExpr},canvas:document.querySelector('.dxgraph')?.getBoundingClientRect().toJSON(),root:${rootQuery}?.getBoundingClientRect().toJSON()})`;
const initial = await wait(snapshotExpr, v => v.nodes > 0 && v.hidden === 0, 'measurement completion');
let focused = initial;
if (semantic) {
    focused = await wait(snapshotExpr, v => v.nodes === 3 && v.hidden === 0 && v.edges === 2, 'Semantic initial expansion');
    const inside = await evaluate(`[...document.querySelectorAll('.dxgraph-node')].every(e=>{const r=e.getBoundingClientRect(),c=document.querySelector('.dxgraph').getBoundingClientRect();return r.left>=c.left&&r.right<=c.right&&r.top>=c.top&&r.bottom<=c.bottom})`);
    assert(inside, 'initial native Semantic fit omitted a node');
} else {
    const focus = await evaluate(`[...document.querySelectorAll('button')].find(e=>e.textContent==='Focus root at 1x').getBoundingClientRect().toJSON()`);
    click(focus.x + focus.width / 2, focus.y + focus.height / 2);
    focused = await wait(snapshotExpr, v => v.transform.includes('scale(1)') && v.nodes < 60 && v.root, 'focused culling');
    assert(Math.abs(focused.root.x + focused.root.width / 2 - focused.canvas.width / 2) < 2, 'controller used the wrong container width');
    assert(Math.abs(focused.root.y + focused.root.height / 2 - focused.canvas.height / 2) < 2, 'controller used the wrong container height');
}
assert(focused.edges > 0, 'expected incident edge paths');
function screenshot(name) { const binary = process.env.MAGICK || 'magick'; execFileSync(binary, ['import', '-window', windowId, resolve(output, name)]); }
screenshot(`${prefix}-focused.png`);
async function timedMoves(x, y, expression, count = 20) {
    move(x, y); input('mousedown', 1); await sleep(120);
    const samples = [];
    for (let index = 1; index <= count; index++) {
        const before = await evaluate(expression); const started = performance.now();
        move(x + index * 7, y + index * 2);
        await wait(expression, value => value !== before, 'gesture DOM update');
        samples.push(performance.now() - started);
    }
    input('mouseup', 1); await sleep(50); return samples;
}
const panSamples = await timedMoves(focused.canvas.x + focused.canvas.width * 0.05, focused.canvas.y + focused.canvas.height * 0.80, transformExpr);
const panAfter = await evaluate(snapshotExpr); assert.notEqual(panAfter.transform, focused.transform);
screenshot(`${prefix}-panned.png`);
let beforeWheel = await evaluate(transformExpr);
move(focused.canvas.x + focused.canvas.width * 0.7, focused.canvas.y + focused.canvas.height * 0.5); input('click', 4);
await wait(transformExpr, value => value !== beforeWheel, 'wheel zoom');
const wheelAfter = await evaluate(transformExpr);
assert.notEqual(wheelAfter, beforeWheel);
const root = await evaluate(`${rootQuery}.getBoundingClientRect().toJSON()`);
const nodeExpr = `${rootQuery}?.style.transform`;
const edgeBefore = await evaluate('document.querySelector(".dxgraph-edge path")?.getAttribute("d")');
const dragSamples = await timedMoves(root.x + root.width / 2, root.y + root.height / 2, nodeExpr);
const edgeAfter = await evaluate('document.querySelector(".dxgraph-edge path")?.getAttribute("d")');
assert.notEqual(edgeAfter, edgeBefore, 'node drag did not update its incident edge');
screenshot(`${prefix}-dragged.png`);
// Exercise input throughput without waiting for each render before sending the next event.
const baseBurst = await evaluate(transformExpr);
const parseTransform = text => { const match = text.match(/translate\(([-.0-9]+)px,\s*([-.0-9]+)px\) scale\(([-.0-9]+)\)/); return match.slice(1).map(Number); };
const [baseX, baseY] = parseTransform(baseBurst);
const bx = focused.canvas.x + focused.canvas.width * 0.05, by = focused.canvas.y + focused.canvas.height * 0.15;
move(bx, by); input('mousedown', 1); await sleep(120);
const burstStarted = performance.now(); let lastSent = burstStarted;
for (let index = 1; index <= 30; index++) {
    const scheduled = burstStarted + index * 1000 / 60;
    await sleep(Math.max(0, scheduled - performance.now()));
    lastSent = performance.now(); move(bx + index * 3, by + index);
}
const expectedX = baseX + (point(bx + 90, by + 30)[0] - point(bx, by)[0]) / geometry.dpr;
const expectedY = baseY + (point(bx + 90, by + 30)[1] - point(bx, by)[1]) / geometry.dpr;
await wait(transformExpr, value => { const [x,y] = parseTransform(value); return Math.abs(x - expectedX) < 1 && Math.abs(y - expectedY) < 1; }, 'final 60 Hz pan sample');
const burst = { events: 30, target_hz: 60, duration_ms: performance.now() - burstStarted, final_input_to_dom_ms: performance.now() - lastSent };
input('mouseup', 1);
let card = null;
if (semantic) {
    const selected = await evaluate(`${rootQuery}.getBoundingClientRect().toJSON()`);
    click(selected.x + selected.width/2, selected.y + selected.height/2);
    card = await wait(`document.querySelector('.semantic-graph-detail')?.innerText`, v=>v?.includes('Desktop graph note'), 'selected EntityCard');
    screenshot('semantic-graph-selected.png');
}
const summary = samples => { const sorted = [...samples].sort((a,b)=>a-b); return { samples: samples.length, median_ms: sorted[Math.floor(sorted.length/2)], p95_ms: sorted[Math.min(sorted.length-1,Math.ceil(sorted.length*0.95)-1)], max_ms: sorted.at(-1), raw_ms: samples }; };
const result = { card, initial, focused, panAfter, wheelAfter, edgeChanged: edgeAfter !== edgeBefore, burst, pan: summary(panSamples), drag: summary(dragSamples), method: 'Real xdotool pointer input to observed WebKitGTK DOM update, including subprocess startup and inspector polling; this is an upper bound on Rust/IPC handling, not pure IPC timing.' };
writeFileSync(resolve(output, semantic ? 'semantic-results.json' : 'results.json'), JSON.stringify(result, null, 2));
console.log(JSON.stringify(result, null, 2)); ws.close();
