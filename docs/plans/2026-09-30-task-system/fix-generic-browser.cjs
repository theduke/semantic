// Additional generic-renderer and normal-note lifecycle coverage.
const {chromium} = require('../../../crates/dxeditor/web/node_modules/playwright');
const assert = require('node:assert/strict');
(async () => {
 const browser = await chromium.launch({headless:true,executablePath:'/etc/profiles/per-user/theduke/bin/chromium',args:['--no-sandbox']});
 const page = await browser.newPage({viewport:{width:1440,height:1000}});
 const errors=[];page.on('pageerror',e=>errors.push(e.message));page.setDefaultTimeout(20000);
 for (const view of ['cards','table']) for (const classId of ['semantic:tasks:task','semantic:comments:comment']) {
  await page.goto(`http://localhost:8080/browse?view=${view}&renderer=table&sql=${encodeURIComponent(`inline:SELECT * FROM entities WHERE type = '${classId}'`)}`);
  await page.locator(view==='table'?'.semantic-entity-list__actions-cell':'.semantic-entity-card').first().waitFor();
  assert.equal(await page.getByRole('button',{name:'Delete entity',exact:true}).count(),0);
  assert(await page.getByRole('button',{name:'Edit labels',exact:true}).count()>0);
 }
 console.log('PASS FALLBACK_RENDERER_CARD_AND_TABLE_ACTIONS');
 await page.goto('http://localhost:8080/notes/create');
 await page.locator('input[type=text]').first().fill('Generic note deletion remains available');
 await page.locator('[contenteditable=true]').first().fill('Normal entity lifecycle fixture.');
 await page.getByRole('button',{name:'Create note',exact:true}).click();
 await page.waitForURL(/\/entities\//);
 await page.getByRole('button',{name:'Delete entity',exact:true}).waitFor();
 const noteId=page.url().split('/').pop();
 await page.getByRole('button',{name:'Delete entity',exact:true}).click();
 await page.getByRole('alertdialog').waitFor();
 await page.getByRole('button',{name:'Cancel',exact:true}).click();
 await page.goto(`http://localhost:8080/browse?view=table&sql=${encodeURIComponent(`inline:SELECT * FROM entities WHERE id = '${noteId}'`)}`);
 await page.getByRole('button',{name:'Delete entity',exact:true}).waitFor();
 assert.deepEqual(errors,[]);
 console.log('PASS GENERIC_NOTE_DELETE_DETAIL_CONFIRMATION_AND_TABLE');
 await browser.close();
})().catch(e=>{console.error(e);process.exit(1)});
