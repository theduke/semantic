use std::pin::Pin;

use semantic_core::api::{self, ApiClient, FileUploadMetadata};
use wasm_bindgen::{JsCast, JsValue};

fn anyerr_from_js(value: wasm_bindgen::JsValue) -> anyhow::Error {
    anyhow::Error::msg(format!("{:?}", value))
}

#[derive(Clone, Debug)]
pub struct BrowserExecutor {
    endpoint: String,
}

impl BrowserExecutor {
    async fn execute(self, query: api::Query) -> Result<api::Reply, anyhow::Error> {
        let body: JsValue = serde_json::to_string(&query)?.into();

        let mut opts = web_sys::RequestInit::new();
        opts.method("POST");
        opts.mode(web_sys::RequestMode::Cors);
        opts.body(Some(&body));

        let request = web_sys::Request::new_with_str_and_init(&self.endpoint, &opts)
            .map_err(anyerr_from_js)?;

        let window = web_sys::window().unwrap();
        let fut1 = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request));
        let out = fut1.await;
        let resp_value = out.map_err(anyerr_from_js)?;

        // `resp_value` is a `Response` object.
        assert!(resp_value.is_instance_of::<web_sys::Response>());
        let resp: web_sys::Response = resp_value.dyn_into().unwrap();

        // Convert this other `Promise` into a rust `Future`.
        let body_js = wasm_bindgen_futures::JsFuture::from(resp.text().map_err(anyerr_from_js)?)
            .await
            .map_err(anyerr_from_js)?
            .dyn_into::<js_sys::JsString>()
            .map_err(anyerr_from_js)?;
        let body: String = body_js.into();

        let response: api::ApiResponse = serde_json::from_slice(body.as_bytes())?;
        match response {
            api::ApiResponse::Ok(reply) => Ok(reply),
            api::ApiResponse::Err(err) => Err(anyhow::Error::msg(err.message)),
        }
    }
}

impl api::ApiClientExecutor for BrowserExecutor {
    type Future = Pin<Box<dyn std::future::Future<Output = Result<api::Reply, anyhow::Error>>>>;

    fn execute(&self, query: api::Query) -> Self::Future {
        Box::pin(self.clone().execute(query))
    }
}

pub type BrowserApiClient = ApiClient<BrowserExecutor>;

pub fn new_api(endpoint: Option<String>) -> BrowserApiClient {
    BrowserApiClient::new(BrowserExecutor {
        endpoint: endpoint.unwrap_or_else(|| "/api/query".to_string()),
    })
}

pub async fn upload_file(
    file: web_sys::File,
    meta: FileUploadMetadata,
) -> Result<api::FileUploadReply, anyhow::Error> {
    let meta_header = base64::encode(serde_json::to_string(&meta)?);

    let mut opts = web_sys::RequestInit::new();
    opts.method("POST");
    opts.mode(web_sys::RequestMode::Cors);
    opts.body(Some(&file));

    // FIXME: generalize URL.
    let url = "/api/upload-file".to_string();
    let request = web_sys::Request::new_with_str_and_init(&url, &opts).map_err(anyerr_from_js)?;
    request
        .headers()
        .set(FileUploadMetadata::HEADER_NAME, &meta_header)
        .unwrap();

    let window = web_sys::window().unwrap();
    let resp_value = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(anyerr_from_js)?;

    // `resp_value` is a `Response` object.
    assert!(resp_value.is_instance_of::<web_sys::Response>());
    let resp: web_sys::Response = resp_value.dyn_into().unwrap();

    // Convert this other `Promise` into a rust `Future`.
    let body_js = wasm_bindgen_futures::JsFuture::from(resp.text().map_err(anyerr_from_js)?)
        .await
        .map_err(anyerr_from_js)?
        .dyn_into::<js_sys::JsString>()
        .map_err(anyerr_from_js)?;
    let body: String = body_js.into();

    let response: api::ApiResponse<api::FileUploadReply> = serde_json::from_slice(body.as_bytes())?;
    match response {
        api::ApiResponse::Ok(reply) => Ok(reply),
        api::ApiResponse::Err(err) => Err(anyhow::Error::msg(err.message)),
    }
}
