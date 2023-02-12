const API_ENDPOINT = "/api/query";

import * as core from "./core";
import { exprEq } from "./db";

import crossFetch from "cross-fetch";

// import { ValueMap } from "./semantic/registry";
import { BaseEntity, FACTOR_ID, SemanticFile, SemanticTag } from "./schema";
import { BlobInfo, FileUploadMetadata, FileUploadReply, Job } from "./core";

export type ValueMap = Record<string, any>;

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

// type QueryOrReply = types.Query | types.Reply;
// type K = keyof QueryOrReply;
type KeysOfUnion<T> = T extends T ? keyof T : never;
type ReplyKeys = KeysOfUnion<core.Reply>;

export class Api {
  host: string;

  constructor(host: string) {
    this.host = host.replace(/\/+$/, "");
  }

  private async fetchApi<R>(key: ReplyKeys, query: core.Query): Promise<R> {
    const res = await crossFetch(this.host + "/api/query", {
      body: JSON.stringify(query),
      method: "POST",
    });
    if (res.status !== 200) {
      // TODO: custom exception
      throw new Error(`Api request failed with status ${res.status}`);
    }
    // TODO: typecheck with yup?
    const responseData: core.ApiResponse = (await res.json()) as any;
    if ("Ok" in responseData) {
      const data = responseData["Ok"];
      if (!(key in (data as object))) {
        throw new Error(
          `API returned malformed response: exptected ${key.toString()}`
        );
      }
      return (data as any)[key];
    } else if ("Err" in responseData) {
      // TODO: custom exception with more info
      throw new Error(responseData.Err.message);
    } else {
      throw new Error("API returned invalid response.");
    }
  }

  async serverStatus(): Promise<core.ServerStatus> {
    return this.fetchApi<core.ServerStatus>("ServerStatus", {
      ServerStatus: null,
    });
  }

  async initialize(config: core.BackendConfig): Promise<core.SemanticSchema> {
    return this.fetchApi<core.SemanticSchema>("Initialize", {
      Initialize: config,
    });
  }

  async closeBackend(): Promise<void> {
    return this.fetchApi<void>("CloseBackend", { CloseBackend: null });
  }

  // TODO: typing for buffer
  async uploadFile(
    buffer: any,
    metadata: FileUploadMetadata
  ): Promise<FileUploadReply> {
    const header = btoa(JSON.stringify(metadata));

    const res = await crossFetch(this.host + "/api/upload-file", {
      method: "POST",
      headers: { ["X-SEMANTIC-FILE-META"]: header },
      body: buffer,
    });
    if (res.status !== 200) {
      throw new Error(`Upload failed with status ${res.status}`);
    }
    const rawData = await res.json();
    if (!("Ok" in rawData)) {
      throw new Error(`API error: ${JSON.stringify(rawData)}`);
    }

    // TODO: validation
    const data = rawData["Ok"];
    return data;
  }

  async select(param: core.Select): Promise<ValueMap[]> {
    return this.fetchApi("Select", { Select: param });
  }

  async selectOne(select: core.Select): Promise<ValueMap | null> {
    const page = await this.select(select);
    return page[0] ?? null;
  }

  async selectSql(query: string): Promise<ValueMap[]> {
    return this.fetchApi("QuerySql", { QuerySql: { query } });
  }

  async selectEntity(query: core.Expr): Promise<ValueMap | null> {
    const page = await this.select({
      ...newSelect(),
      filter: query,
    });
    return page[0] ?? null;
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
    return this.fetchApi("Mutate", { Mutate: param });
  }

  async batch(param: core.Batch): Promise<void> {
    return this.fetchApi("Batch", { Batch: param });
  }

  async schema(): Promise<core.SemanticSchema> {
    return this.fetchApi("Schema", { Schema: null });
  }

  async pluginSourceCreate(
    param: core.PluginSource
  ): Promise<core.PluginSource> {
    return this.fetchApi("PluginSourceCreate", { PluginSourceCreate: param });
  }

  async pluginSourceUpdate(
    param: core.PluginSource
  ): Promise<core.PluginSource> {
    return this.fetchApi("PluginSourceUpgrade", { PluginSourceUpdate: param });
  }

  async pluginSourceValidate(
    param: core.PluginSource
  ): Promise<core.PluginSource> {
    return this.fetchApi("PluginSourceValidate", {
      PluginSourceValidate: param,
    });
  }

  async pluginDelete(param: core.PluginDelete): Promise<void> {
    return this.fetchApi("PluginDelete", { PluginDelete: param });
  }

  async pluginTestFetch(
    param: core.PluginTestFetch
  ): Promise<core.FetchUrlOutput | null> {
    return this.fetchApi("PluginTestFetch", { PluginTestFetch: param });
  }

  async import(param: core.ImportJob): Promise<core.ImportOutput> {
    return this.fetchApi("Import", { Import: param });
  }

  async fetchUrl(param: core.FetchUrlJob): Promise<core.FetchUrlOutput> {
    return this.fetchApi("FetchUrl", { FetchUrl: param });
  }

  async optimiseVideo(
    param: core.OptimiseVideo
  ): Promise<core.OptimiseVideoReply> {
    return this.fetchApi("OptimiseVideo", { OptimiseVideo: param });
  }

  async fileDiscardUnOptimized(
    param: core.FileDiscardUnOptimized
  ): Promise<void> {
    return this.fetchApi("FileDiscardOptimised", {
      FileDiscardUnOptimized: param,
    });
  }

  async fileDiscardOptimised(
    param: core.FileDiscardUnOptimized
  ): Promise<void> {
    return this.fetchApi("FileDiscardOptimised", {
      FileDiscardOptimised: param,
    });
  }

  async fileCreatePreviewImageBlob(
    param: core.FileCreatePreviewImageBlob
  ): Promise<void> {
    return this.fetchApi("FileCreatePreviewImageBlob", {
      FileCreatePreviewImageBlob: param,
    });
  }

  async httpFetch(
    param: core.SimpleHttpRequest
  ): Promise<core.SimpleHttpResponse> {
    return this.fetchApi("HttpFetch", { HttpFetch: param });
  }

  async jobStatus(param: string): Promise<core.Job> {
    const job = await this.fetchApi<core.Job>("JobStatus", {
      JobStatus: param,
    });
    console.log({ apiJob: job });
    return job;
  }

  async jobEvents(jobId: string): Promise<core.JobEvent[]> {
    const events = await this.fetchApi<core.JobEvent[]>("JobEvents", {
      JobEvents: jobId,
    });
    return events;
  }

  async convertFile(param: core.ConvertFile): Promise<core.Job> {
    return this.fetchApi("ConvertFile", { ConvertFile: param });
  }

  // FIXME: change APi to the return here is a custom struct instead of a plain enum variant
  async findUnusedBlobs(): Promise<BlobInfo[]> {
    const out: { items: BlobInfo[] } = await this.fetchApi("FindUnusedBlobs", {
      FindUnusedBlobs: null,
    });
    return out.items;
  }

  async deleteUnusedBlobs(): Promise<core.UnusedBlobsDeleted> {
    return this.fetchApi("DeleteUnusedBlobs", { DeleteUnusedBlobs: null });
  }

  async analyzeMedia(param: { force: boolean }): Promise<void> {
    return this.fetchApi("AnalyzeMedia", { AnalyzeMedia: param });
  }

  async tagCreate(param: core.TagCreate): Promise<SemanticTag> {
    // TODO: type check of returned data?
    return this.fetchApi("TagCreate", { TagCreate: param });
  }

  async tagMerge(source: string, target: string): Promise<void> {
    // TODO: type check of returned data?
    return this.fetchApi("TagMerge", {
      TagMerge: { target_tag: target, source_tag: source },
    });
  }

  async startMediaAnalysis(): Promise<Job> {
    // TODO: type check of returned data?
    return this.fetchApi("AnalyzeMedia", { AnalyzeMedia: { force: false } });
  }

  async findSimilarImages(
    maxCount: number,
    similarityMin: number,
    similarityMax: number
  ): Promise<Job> {
    // TODO: type check of returned data?
    return this.fetchApi("FindSimilarImages", {
      FindSimilarImages: {
        max_results: maxCount as any,
        similarity_min: similarityMin,
        similarity_max: similarityMax,
      },
    });
  }
}
