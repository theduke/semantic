pub mod api;
pub mod context;
pub mod plugin;
pub mod registry;
pub mod routing;

pub mod base;

pub mod validate;

pub mod components;

pub type SharedRenderer0 = std::rc::Rc<dyn Fn() -> TagBuilder>;

use brass::dom::TagBuilder;
use factordb::data::Timestamp;
use js_sys::JsString;
use wasm_bindgen::JsValue;

pub use self::{
    plugin::{BrowserPlugin, BrowserPluginSpec},
    registry::{
        DynEntityRenderer, EntityInfo, EntityRenderMode, EntityRenderOpts, EntityRendererSpec,
        Registry, SharedRegistry,
    },
};

pub fn now() -> Timestamp {
    Timestamp::from_millis(js_sys::Date::now().round() as u64)
}

pub fn datetime_to_locale_string_js(dt: &chrono::DateTime<chrono::Utc>) -> JsString {
    js_sys::Date::new(&JsValue::from(dt.timestamp_millis())).to_locale_string("", &JsValue::NULL)
}
