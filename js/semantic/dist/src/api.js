"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.Api = exports.newSelect = void 0;
const API_ENDPOINT = "/api/query";
const db_1 = require("./db");
const cross_fetch_1 = __importDefault(require("cross-fetch"));
// import { ValueMap } from "./semantic/registry";
const schema_1 = require("./schema");
function newSelect() {
    return {
        filter: null,
        // FIXME: change type defs to use number instead of bigint
        limit: 50,
        offset: 0,
        cursor: null,
        variables: {},
        aggregate: [],
        sort: [],
        joins: [],
    };
}
exports.newSelect = newSelect;
class Api {
    host;
    constructor(host) {
        this.host = host.replace(/\/+$/, "");
    }
    async fetchApi(key, query) {
        const res = await (0, cross_fetch_1.default)(this.host + '/api/query', {
            body: JSON.stringify(query),
            method: "POST",
        });
        if (res.status !== 200) {
            // TODO: custom exception
            throw new Error(`Api request failed with status ${res.status}`);
        }
        // TODO: typecheck with yup?
        const responseData = (await res.json());
        if ("Ok" in responseData) {
            const data = responseData['Ok'];
            if (!(key in data)) {
                throw new Error(`API returned malformed response: exptected ${key.toString()}`);
            }
            return data[key];
        }
        else if ("Err" in responseData) {
            // TODO: custom exception with more info
            throw new Error(responseData.Err.message);
        }
        else {
            throw new Error("API returned invalid response.");
        }
    }
    async serverStatus() {
        return this.fetchApi("ServerStatus", { ServerStatus: null });
    }
    async initialize(config) {
        return this.fetchApi("Initialize", { Initialize: config });
    }
    async closeBackend() {
        return this.fetchApi("CloseBackend", { CloseBackend: null });
    }
    async uploadFile(buffer, metadata) {
        const header = btoa(JSON.stringify(metadata));
        const res = await (0, cross_fetch_1.default)(this.host + '/api/upload-file', {
            method: 'POST',
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
        const data = rawData['Ok'];
        return data;
    }
    async select(param) {
        return this.fetchApi("Select", { Select: param });
    }
    async selectOne(select) {
        const page = await this.select(select);
        return page[0] ?? null;
    }
    async selectSql(query) {
        return this.fetchApi("QuerySql", { QuerySql: { query } });
    }
    async entity(ident) {
        const page = await this.select({
            ...newSelect(),
            filter: (0, db_1.exprEq)({ Attr: schema_1.FACTOR_ID }, { Ident: ident }),
        });
        const item = page[0];
        if (!item) {
            throw new Error(`Entity ${ident} not found`);
        }
        return item;
    }
    async mutate(param) {
        return this.fetchApi("Mutate", { Mutate: param });
    }
    async batch(param) {
        return this.fetchApi("Batch", { Batch: param });
    }
    async schema() {
        return this.fetchApi("Schema", { Schema: null });
    }
    async pluginSourceCreate(param) {
        return this.fetchApi("PluginSourceCreate", { PluginSourceCreate: param });
    }
    async pluginSourceUpdate(param) {
        return this.fetchApi("PluginSourceUpgrade", { PluginSourceUpdate: param });
    }
    async pluginSourceValidate(param) {
        return this.fetchApi("PluginSourceValidate", { PluginSourceValidate: param });
    }
    async pluginDelete(param) {
        return this.fetchApi("PluginDelete", { PluginDelete: param });
    }
    async pluginTestFetch(param) {
        return this.fetchApi("PluginTestFetch", { PluginTestFetch: param });
    }
    async import(param) {
        return this.fetchApi("Import", { Import: param });
    }
    async fetchUrl(param) {
        return this.fetchApi("FetchUrl", { FetchUrl: param });
    }
    async optimiseVideo(param) {
        return this.fetchApi("OptimiseVideo", { OptimiseVideo: param });
    }
    async fileDiscardUnOptimized(param) {
        return this.fetchApi("FileDiscardOptimised", { FileDiscardUnOptimized: param });
    }
    async fileDiscardOptimised(param) {
        return this.fetchApi("FileDiscardOptimised", { FileDiscardOptimised: param });
    }
    async fileCreatePreviewImageBlob(param) {
        return this.fetchApi("FileCreatePreviewImageBlob", {
            FileCreatePreviewImageBlob: param,
        });
    }
    async httpFetch(param) {
        return this.fetchApi("HttpFetch", { HttpFetch: param });
    }
    async jobStatus(param) {
        return this.fetchApi("JobStatus", { JobStatus: param });
    }
    async convertFile(param) {
        return this.fetchApi("ConvertFile", { ConvertFile: param });
    }
    // FIXME: change APi to the return here is a custom struct instead of a plain enum variant
    async findUnusedBlobs() {
        return this.fetchApi("FindUnusedBlobs", { FindUnusedBlobs: null });
    }
    async deleteUnusedBlobs() {
        return this.fetchApi("DeleteUnusedBlobs", { DeleteUnusedBlobs: null });
    }
    async analyzeMedia(param) {
        return this.fetchApi("AnalyzeMedia", { AnalyzeMedia: param });
    }
    async tagCreate(param) {
        // TODO: type check of returned data?
        return this.fetchApi("TagCreate", { TagCreate: param });
    }
}
exports.Api = Api;
