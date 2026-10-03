//! Optional pointer capture; native renderers finish gestures on pointerleave.
use dioxus::prelude::MountedData;

pub fn capture(element: Option<&MountedData>, pointer: i32) {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    if let Some(element) = element.and_then(|data| data.downcast::<web_sys::Element>()) {
        let _ = element.set_pointer_capture(pointer);
    }
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    let _ = (element, pointer);
}
pub fn release(element: Option<&MountedData>, pointer: i32) {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    if let Some(element) = element.and_then(|data| data.downcast::<web_sys::Element>()) {
        let _ = element.release_pointer_capture(pointer);
    }
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    let _ = (element, pointer);
}
pub fn timestamp_ms() -> u64 {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        web_sys::window()
            .and_then(|window| window.performance())
            .map(|clock| clock.now() as u64)
            .unwrap_or(0)
    }
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis() as u64
    }
}
