import browser from "webextension-polyfill";

import { Api, newSelect } from 'semantic/dist/api';

import { v4 as uuidv4 } from 'uuid';
import { exprAnd, exprAttr, exprIn, exprIsEntityType, exprList, exprLiteral } from "semantic/dist/db";
import { FACTOR_ID, FACTOR_TYPE, SemanticBookmark, SEMANTIC_TITLE, SEMANTIC_URL, TY_SEMANTIC_BOOKMARK } from "semantic/dist/schema";

browser.runtime.onInstalled.addListener(() => {
  console.log("Extension installed");
});

interface Config {
  serverUrl: string;
}

const STORAGE_KEY_CONFIG = 'semantic_config';

async function loadConfig(): Promise<Config> {
  let config = await browser.storage.local.get(STORAGE_KEY_CONFIG);
  // TODO: validation
  if (!config || !config[STORAGE_KEY_CONFIG]) {
    config = { serverUrl: 'http://localhost:3000' };
  }
  return config as any;
}

class SemanticExt {
  config: Config;
  api: Api;

  constructor(config: Config) {
    this.config = config;
    this.api = new Api(config.serverUrl);
  }

  static async create() {
    const config = await loadConfig();
    return new SemanticExt(config);
  }
}

interface FlatBookmark {
  id: string;
  parentId?: string;
  url: string;
  title?: string;
}

function flattenBookmarksTree(node: browser.Bookmarks.BookmarkTreeNode, collector: FlatBookmark[]) {
  switch (node.type) {
    case 'bookmark':
      const title = node.title;
      const url = node.url;
      if (url) {
        const flat: FlatBookmark = {
          id: node.id,
          parentId: node.parentId,
          url,
          title: title || undefined,
        };
        collector.push(flat);
      }

      break;
    case 'folder':
      node.children?.forEach(child => {
        flattenBookmarksTree(child, collector);
      });
      break;
    case 'separator':
      // Ignored.
      break;
    default:
      throw new Error('unhandled bookmark type: ' + node.type);
  }
}

async function syncBookmarks(api: Api) {
  console.trace('starting bookmark sync...!');

  // Sync local to semantic server.
  const tree = await browser.bookmarks.getTree();
  const flat: FlatBookmark[] = [];
  flattenBookmarksTree(tree[0], flat);

  console.log(`syncing local bookmarks to remote`, { tree, flat });

  const handledUrls: Set<string> = new Set();

  for (const bookmark of flat) {
    // NOTE: firefox appends a trailing slash to bookmark urls,
    // so we need to cover that case to avoid duplicates.

    const noTrailing = bookmark.url.endsWith('/') ? bookmark.url.slice(0, -1) : bookmark.url;
    const trailing = bookmark.url.endsWith('/') ? bookmark.url : bookmark.url + '/';

    const filter = exprAnd(
      exprIsEntityType(TY_SEMANTIC_BOOKMARK),
      exprIn(exprAttr(SEMANTIC_URL), exprList([exprLiteral(trailing), exprLiteral(noTrailing)]))
    );
    const oldBookmark = await api.selectEntity(filter);

    if (oldBookmark) {
      console.log('found existing bookmark', { bookmark, oldBookmark });
      if (oldBookmark[SEMANTIC_TITLE] != bookmark.title) {
        const data = { [SEMANTIC_TITLE]: bookmark.title };
        console.log('updating existing bookmark', { bookmark, oldBookmark, newData: data });
        await api.batch({ actions: [{ Merge: { id: oldBookmark[FACTOR_ID], data } }] });
      }
    } else {
      const id = uuidv4();
      const b: SemanticBookmark = {
        [FACTOR_ID]: id,
        [FACTOR_TYPE]: TY_SEMANTIC_BOOKMARK,
        [SEMANTIC_URL]: bookmark.url,
      };
      if (bookmark.title) {
        b[SEMANTIC_TITLE] = bookmark.title;
      }
      await api.batch({ actions: [{ Create: { id, data: b } }] });
      console.log('semantic bookmark created', b);
    }

    handledUrls.add(trailing);
    handledUrls.add(noTrailing);
  }

  console.log('syncing remote bookmarks to local', { handledUrls });

  let offset = 0;
  let createdCounter = 0;
  while (true) {
    const page = await api.select({
      ...newSelect(),
      filter: exprIsEntityType(TY_SEMANTIC_BOOKMARK),
      offset: offset as any,
      limit: 1000 as any,
    });

    if (page.length === 0) {
      break;
    }
    console.log({ page })

    for (const item of page) {
      const mark = item as SemanticBookmark;
      const url = mark[SEMANTIC_URL];
      const title = mark[SEMANTIC_TITLE] || undefined;

      if (handledUrls.has(url)) {
        continue;
      }

      const data: browser.Bookmarks.CreateDetails = {
        type: 'bookmark',
        parentId: 'unfiled_____',
        title,
        url,
      };
      console.log('creating bookmark', data);
      await browser.bookmarks.create(data);
      createdCounter += 1;
      handledUrls.add(url);
    }

    offset += page.length;
  }

  console.log(`imported ${createdCounter} bookmarks from Semantic`);


  console.log('bookmarks sync complete');

  console.log(api)
}

(window as any).syncBookmarks = syncBookmarks;

async function start() {
  console.log('starting extension...');
  const ext = await SemanticExt.create();
  await syncBookmarks(ext.api);
}

console.log('extension!')
start();


(window as any).start = start;
