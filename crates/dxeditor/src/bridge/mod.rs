use dioxus::prelude::*;

use crate::{InputEvent, selection::EditorSelection};

const SCRIPT: &str = include_str!("editor_bridge.js");

#[component]
pub(crate) fn EditorBridge(editor_id: String, on_event: EventHandler<InputEvent>) -> Element {
    use_effect(move || {
        let script = SCRIPT.replace("__EDITOR_ID__", &editor_id);
        spawn(async move {
            let mut eval = document::eval(&script);
            while let Ok(event) = eval.recv::<InputEvent>().await {
                on_event.call(event);
            }
        });
    });
    rsx! {}
}

pub(crate) fn set_dom_selection(editor_id: &str, selection: &EditorSelection) {
    let Ok(selection) = serde_json::to_string(selection) else {
        return;
    };
    let Ok(editor_id) = serde_json::to_string(editor_id) else {
        return;
    };
    let script = format!(
        r#"
const root = document.querySelector('[data-dxeditor-id="' + {editor_id} + '"]');
const requested = {selection};
function locate(blockId, charOffset) {{
  const block = root?.querySelector(`[data-block-id="${{CSS.escape(blockId)}}"]`);
  if (!block) return null;
  const walker = document.createTreeWalker(block, NodeFilter.SHOW_TEXT);
  let remaining = charOffset, node;
  while ((node = walker.nextNode())) {{
    const values = Array.from(node.data);
    if (remaining <= values.length) return [node, values.slice(0, remaining).join('').length];
    remaining -= values.length;
  }}
  return [block, block.childNodes.length];
}}
const anchor = locate(requested.anchor.block_id, requested.anchor.offset);
const focus = locate(requested.focus.block_id, requested.focus.offset);
if (anchor && focus) {{
  const value = document.getSelection(); value.removeAllRanges();
  value.setBaseAndExtent(anchor[0], anchor[1], focus[0], focus[1]);
  root.__dxeditorBridge?.acknowledgeInput();
}} else if (root) {{
  root.__dxeditorBridge?.acknowledgeInput();
}}
"#
    );
    let _ = document::eval(&script);
}
