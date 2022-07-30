/// <reference types="node" />
import * as core from "./core";
import { SemanticTag } from "./schema";
import { FileUploadMetadata, FileUploadReply } from "./core";
export declare type ValueMap = Record<string, any>;
export declare function newSelect(): core.Select;
export declare class Api {
    host: string;
    constructor(host: string);
    private fetchApi;
    serverStatus(): Promise<core.ServerStatus>;
    initialize(config: core.BackendConfig): Promise<core.SemanticSchema>;
    closeBackend(): Promise<void>;
    uploadFile(buffer: Buffer, metadata: FileUploadMetadata): Promise<FileUploadReply>;
    select(param: core.Select): Promise<ValueMap[]>;
    selectOne(select: core.Select): Promise<ValueMap | null>;
    selectSql(query: string): Promise<ValueMap[]>;
    entity(ident: core.IdOrIdent): Promise<ValueMap>;
    mutate(param: core.Mutate): Promise<void>;
    batch(param: core.Batch): Promise<void>;
    schema(): Promise<core.SemanticSchema>;
    pluginSourceCreate(param: core.PluginSource): Promise<core.PluginSource>;
    pluginSourceUpdate(param: core.PluginSource): Promise<core.PluginSource>;
    pluginSourceValidate(param: core.PluginSource): Promise<core.PluginSource>;
    pluginDelete(param: core.PluginDelete): Promise<void>;
    pluginTestFetch(param: core.PluginTestFetch): Promise<core.FetchUrlOutput | null>;
    import(param: core.ImportJob): Promise<core.ImportOutput>;
    fetchUrl(param: core.FetchUrlJob): Promise<core.FetchUrlOutput>;
    optimiseVideo(param: core.OptimiseVideo): Promise<core.OptimiseVideoReply>;
    fileDiscardUnOptimized(param: core.FileDiscardUnOptimized): Promise<void>;
    fileDiscardOptimised(param: core.FileDiscardUnOptimized): Promise<void>;
    fileCreatePreviewImageBlob(param: core.FileCreatePreviewImageBlob): Promise<void>;
    httpFetch(param: core.SimpleHttpRequest): Promise<core.SimpleHttpResponse>;
    jobStatus(param: string): Promise<core.JobStatus>;
    convertFile(param: core.ConvertFile): Promise<core.Job>;
    findUnusedBlobs(): Promise<{
        items: Array<core.BlobInfo>;
    }>;
    deleteUnusedBlobs(): Promise<core.UnusedBlobsDeleted>;
    analyzeMedia(param: {
        force: boolean;
    }): Promise<void>;
    tagCreate(param: core.TagCreate): Promise<SemanticTag>;
}
