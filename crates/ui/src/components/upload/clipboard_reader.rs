use brass::{dom::TagBuilder, effect::EventSubscription};
use semantic_ui_core::components::util::notification_default;

// Clipboard API resources:
// * https://web.dev/async-clipboard/: has examples and explanations
pub fn clipboard_reader(_on_paste: impl Fn(Vec<web_sys::File>) + 'static) -> TagBuilder {
    let sub = EventSubscription::subscribe(
        web_sys::window().unwrap().into(),
        brass::dom::Ev::Paste,
        move |_ev: web_sys::Event| {},
        // NOTE: currently disabled due to ClipboardEvent being experimental
        // (it requires a custom --cfg RUSTFLAG)
        // move |ev: web_sys::ClipboardEvent| {
        //     if let Some(files) = ev.clipboard_data().and_then(|d| d.files()) {
        //         let items: Vec<web_sys::File> = (0..files.length())
        //             .filter_map(|index| files.item(index))
        //             .collect();
        //         if !items.is_empty() {
        //             on_paste(items);
        //         }
        //     }
        // },
    );
    notification_default()
        .bind(sub)
        .and("Paste files from the clipboard with CTRL-V")
}
