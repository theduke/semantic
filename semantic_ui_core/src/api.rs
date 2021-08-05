use std::pin::Pin;

use factordb::AnyError;
use semantics_core::api::ApiClient;
use wasm_bindgen::{JsCast, JsValue};

fn anyerr_from_js(value: wasm_bindgen::JsValue) -> AnyError {
    AnyError::msg(format!("{:?}", value))
}

#[derive(Clone, Copy, Debug)]
pub struct BrowserExecutor;

impl BrowserExecutor {
    async fn execute(
        self,
        query: semantics_core::api::Query,
    ) -> Result<semantics_core::api::Reply, AnyError> {
        let body: JsValue = serde_json::to_string(&query)?.into();

        let mut opts = web_sys::RequestInit::new();
        opts.method("POST");
        opts.mode(web_sys::RequestMode::Cors);
        opts.body(Some(&body));

        let url = "http://localhost:3000/api/query".to_string();
        let request =
            web_sys::Request::new_with_str_and_init(&url, &opts).map_err(anyerr_from_js)?;

        // request
        //     .headers()
        //     .set("Accept", "application/vnd.github.v3+json");

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

        let response: semantics_core::api::ApiResponse = serde_json::from_slice(body.as_bytes())?;
        match response {
            semantics_core::api::ApiResponse::Ok(reply) => Ok(reply),
            semantics_core::api::ApiResponse::Err(err) => Err(AnyError::msg(err.message)),
        }
    }
}

impl semantics_core::api::ApiClientExecutor for BrowserExecutor {
    type Future =
        Pin<Box<dyn std::future::Future<Output = Result<semantics_core::api::Reply, AnyError>>>>;

    fn execute(&self, query: semantics_core::api::Query) -> Self::Future {
        Box::pin(self.clone().execute(query))
    }
}

pub type BrowserApiClient = ApiClient<BrowserExecutor>;

pub fn api() -> BrowserApiClient {
    BrowserApiClient::new(BrowserExecutor)
}
