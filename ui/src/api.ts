const API_ENDPOINT = "/api/query";

import * as core from "./semantic/core";
import { exprEq } from "./semantic/db";
import { ValueMap } from "./semantic/registry";
import { FACTOR_ID, SemanticTag } from "./semantic/schema";

export function newSelect(): core.Select {
  return {
    filter: null,
    // FIXME: change type defs to use number instead of bigint
    limit: 50 as any,
    offset: 0 as any,
    cursor: null,
    variables: {},
    aggregate: [],
    sort: [],
    joins: [],
  };
}

async function fetchApiRaw(payload: core.Query): Promise<core.Reply> {
  const res = await fetch(API_ENDPOINT, {
    body: JSON.stringify(payload),
    method: "POST",
  });
  if (res.status !== 200) {
    // TODO: custom exception
    throw new Error(`Api request failed with status ${res.status}`);
  }
  // TODO: typecheck with yup?
  const data: core.ApiResponse = (await res.json()) as any;
  if ("Ok" in data) {
    return data.Ok;
  } else if ("Err" in data) {
    // TODO: custom exception with more info
    throw new Error(data.Err.message);
  } else {
    throw new Error("API returned invalid response.");
  }
}

// type QueryOrReply = types.Query | types.Reply;
// type K = keyof QueryOrReply;
type KeysOfUnion<T> = T extends T ? keyof T : never;
type ReplyKeys = KeysOfUnion<core.Reply>;

async function fetchApi<R>(key: ReplyKeys, query: core.Query): Promise<R> {
  const reply = await fetchApiRaw(query);
  if (!(key in (reply as object))) {
    throw new Error(
      `API returned malformed response: exptected ${key.toString()}`
    );
  }
  return (reply as any)[key];
}

export class Api {
  async serverStatus(): Promise<core.ServerStatus> {
    return fetchApi<core.ServerStatus>("ServerStatus", { ServerStatus: null });
  }

  async initialize(config: core.BackendConfig): Promise<core.SemanticSchema> {
    return fetchApi<core.SemanticSchema>("Initialize", { Initialize: config });
  }

  async closeBackend(): Promise<void> {
    return fetchApi<void>("CloseBackend", { CloseBackend: null });
  }

  async select(param: core.Select): Promise<ValueMap[]> {
    return fetchApi("Select", { Select: param });
  }

  async entity(ident: core.IdOrIdent): Promise<ValueMap> {
    const page = await this.select({
      ...newSelect(),
      filter: exprEq({ Attr: FACTOR_ID }, { Ident: ident }),
    });

    const item = page[0];
    if (!item) {
      throw new Error(`Entity ${ident} not found`);
    }
    return item;
  }

  async mutate(param: core.Mutate): Promise<void> {
    return fetchApi("Mutate", { Mutate: param });
  }

  async batch(param: core.Batch): Promise<void> {
    return fetchApi("Batch", { Batch: param });
  }

  async schema(): Promise<core.SemanticSchema> {
    return fetchApi("Schema", { Schema: null });
  }

  async pluginSourceCreate(
    param: core.PluginSource
  ): Promise<core.PluginSource> {
    return fetchApi("PluginSourceCreate", { PluginSourceCreate: param });
  }

  async pluginSourceUpdate(
    param: core.PluginSource
  ): Promise<core.PluginSource> {
    return fetchApi("PluginSourceUpgrade", { PluginSourceUpdate: param });
  }

  async pluginSourceValidate(
    param: core.PluginSource
  ): Promise<core.PluginSource> {
    return fetchApi("PluginSourceValidate", { PluginSourceValidate: param });
  }

  async pluginDelete(param: core.PluginDelete): Promise<void> {
    return fetchApi("PluginDelete", { PluginDelete: param });
  }

  async pluginTestFetch(
    param: core.PluginTestFetch
  ): Promise<core.FetchUrlOutput | null> {
    return fetchApi("PluginTestFetch", { PluginTestFetch: param });
  }

  async import(param: core.ImportJob): Promise<core.ImportOutput> {
    return fetchApi("Import", { Import: param });
  }

  async fetchUrl(param: core.FetchUrlJob): Promise<core.FetchUrlOutput> {
    return fetchApi("FetchUrl", { FetchUrl: param });
  }

  async optimiseVideo(
    param: core.OptimiseVideo
  ): Promise<core.OptimiseVideoReply> {
    return fetchApi("OptimiseVideo", { OptimiseVideo: param });
  }

  async fileDiscardUnOptimized(
    param: core.FileDiscardUnOptimized
  ): Promise<void> {
    return fetchApi("FileDiscardOptimised", { FileDiscardUnOptimized: param });
  }

  async fileDiscardOptimised(
    param: core.FileDiscardUnOptimized
  ): Promise<void> {
    return fetchApi("FileDiscardOptimised", { FileDiscardOptimised: param });
  }

  async fileCreatePreviewImageBlob(
    param: core.FileCreatePreviewImageBlob
  ): Promise<void> {
    return fetchApi("FileCreatePreviewImageBlob", {
      FileCreatePreviewImageBlob: param,
    });
  }

  async httpFetch(
    param: core.SimpleHttpRequest
  ): Promise<core.SimpleHttpResponse> {
    return fetchApi("HttpFetch", { HttpFetch: param });
  }

  async jobStatus(param: string): Promise<core.JobStatus> {
    return fetchApi("JobStatus", { JobStatus: param });
  }

  async convertFile(param: core.ConvertFile): Promise<core.Job> {
    return fetchApi("ConvertFile", { ConvertFile: param });
  }

  // FIXME: change APi to the return here is a custom struct instead of a plain enum variant
  async findUnusedBlobs(): Promise<{ items: Array<core.BlobInfo> }> {
    return fetchApi("FindUnusedBlobs", { FindUnusedBlobs: null });
  }

  async deleteUnusedBlobs(): Promise<core.UnusedBlobsDeleted> {
    return fetchApi("DeleteUnusedBlobs", { DeleteUnusedBlobs: null });
  }

  async analyzeMedia(param: { force: boolean }): Promise<void> {
    return fetchApi("AnalyzeMedia", { AnalyzeMedia: param });
  }

  async tagCreate(param: core.TagCreate): Promise<SemanticTag> {
    // TODO: type check of returned data?
    return fetchApi("TagCreate", { TagCreate: param });
  }
}
