use crate::selection::EditorSelection;

#[cfg(all(feature = "web", target_arch = "wasm32"))]
use crate::selection::TextPosition;

#[cfg(feature = "web")]
const BROWSER_SELECTION_SCRIPT: &str = r#"
function ancestorWithAttr(node, attr) {
  let current = node && node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement;
  while (current) {
    if (current.hasAttribute && current.hasAttribute(attr)) {
      return current;
    }
    current = current.parentElement;
  }
  return null;
}

function parseIntAttr(element, attr) {
  const value = Number.parseInt(element.getAttribute(attr) || "0", 10);
  return Number.isFinite(value) ? value : 0;
}

function textLenBetween(container, endNode, endOffset) {
  const range = document.createRange();
  range.setStart(container, 0);
  range.setEnd(endNode, endOffset);
  return Array.from(range.toString()).length;
}

function renderedLocalToModelOffset(renderedLocalOffset, renderedPrefixLen, modelLen) {
  return Math.min(Math.max(renderedLocalOffset - renderedPrefixLen, 0), modelLen);
}

function domPointToTextPosition(node, offset) {
  const block = ancestorWithAttr(node, "data-block-id");
  if (!block) {
    return null;
  }

  const blockLen = parseIntAttr(block, "data-text-len");
  const inline = ancestorWithAttr(node, "data-inline-id");
  let absoluteOffset = 0;

  if (inline) {
    const inlineStart = parseIntAttr(inline, "data-inline-start");
    const modelLen = parseIntAttr(inline, "data-inline-text-len");
    const prefixLen = parseIntAttr(inline, "data-inline-prefix-len");
    const renderedLocalOffset = textLenBetween(inline, node, offset);
    absoluteOffset = Math.min(
      inlineStart + renderedLocalToModelOffset(renderedLocalOffset, prefixLen, modelLen),
      blockLen
    );
  } else {
    absoluteOffset = Math.min(textLenBetween(block, node, offset), blockLen);
  }

  return {
    block_id: block.getAttribute("data-block-id"),
    inline_id: null,
    offset: absoluteOffset
  };
}

const selection = document.getSelection();
if (!selection || selection.rangeCount === 0 || !selection.anchorNode || !selection.focusNode) {
  return null;
}

const anchor = domPointToTextPosition(selection.anchorNode, selection.anchorOffset);
const focus = domPointToTextPosition(selection.focusNode, selection.focusOffset);
if (!anchor || !focus) {
  return null;
}

return { anchor, focus };
"#;

#[cfg(any(test, all(feature = "web", target_arch = "wasm32")))]
pub(crate) fn rendered_local_to_model_offset(
    rendered_local_offset: usize,
    rendered_prefix_len: usize,
    model_len: usize,
) -> usize {
    rendered_local_offset
        .saturating_sub(rendered_prefix_len)
        .min(model_len)
}

#[cfg(any(test, all(feature = "web", target_arch = "wasm32")))]
pub(crate) fn absolute_model_offset(
    inline_start: usize,
    rendered_local_offset: usize,
    rendered_prefix_len: usize,
    model_len: usize,
    block_len: usize,
) -> usize {
    inline_start
        .saturating_add(rendered_local_to_model_offset(
            rendered_local_offset,
            rendered_prefix_len,
            model_len,
        ))
        .min(block_len)
}

#[cfg(feature = "web")]
pub(crate) async fn browser_selection() -> Option<EditorSelection> {
    #[cfg(target_arch = "wasm32")]
    if let Some(selection) = web_sys_browser_selection() {
        return Some(selection);
    }

    dioxus::document::eval(BROWSER_SELECTION_SCRIPT)
        .join::<Option<EditorSelection>>()
        .await
        .ok()
        .flatten()
}

#[cfg(not(feature = "web"))]
pub(crate) async fn browser_selection() -> Option<EditorSelection> {
    None
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn web_sys_browser_selection() -> Option<EditorSelection> {
    let selection = web_sys::window()?
        .document()?
        .get_selection()
        .ok()
        .flatten()?;
    if selection.range_count() == 0 {
        return None;
    }

    Some(EditorSelection {
        anchor: dom_point_to_text_position(selection.anchor_node()?, selection.anchor_offset())?,
        focus: dom_point_to_text_position(selection.focus_node()?, selection.focus_offset())?,
    })
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn dom_point_to_text_position(node: web_sys::Node, offset: u32) -> Option<TextPosition> {
    let block_element = ancestor_with_attr(&node, "data-block-id")?;
    let block_id = block_element.get_attribute("data-block-id")?;
    let block_len = parse_attr_usize(&block_element, "data-text-len");

    if let Some(inline_element) = ancestor_with_attr(&node, "data-inline-id") {
        let inline_start = parse_attr_usize(&inline_element, "data-inline-start");
        let model_len = parse_attr_usize(&inline_element, "data-inline-text-len");
        let rendered_prefix_len = parse_attr_usize(&inline_element, "data-inline-prefix-len");
        let rendered_local_offset =
            text_len_between(&inline_element.clone().into(), &node, offset)?;
        let absolute_offset = absolute_model_offset(
            inline_start,
            rendered_local_offset,
            rendered_prefix_len,
            model_len,
            block_len,
        );

        return Some(TextPosition::new(block_id, None, absolute_offset));
    }

    let absolute_offset =
        text_len_between(&block_element.clone().into(), &node, offset)?.min(block_len);
    Some(TextPosition::new(block_id, None, absolute_offset))
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn ancestor_with_attr(node: &web_sys::Node, attr: &str) -> Option<web_sys::Element> {
    use wasm_bindgen::JsCast;

    let mut current = Some(node.clone());
    while let Some(candidate) = current {
        if let Some(element) = candidate.dyn_ref::<web_sys::Element>() {
            if element.has_attribute(attr) {
                return Some(element.clone());
            }
        }
        current = candidate.parent_node();
    }
    None
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn text_len_between(
    start_container: &web_sys::Node,
    end_node: &web_sys::Node,
    end_offset: u32,
) -> Option<usize> {
    let range = web_sys::window()?.document()?.create_range().ok()?;
    range.set_start(start_container, 0).ok()?;
    range.set_end(end_node, end_offset).ok()?;
    Some(String::from(range.to_string()).chars().count())
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn parse_attr_usize(element: &web_sys::Element, attr: &str) -> usize {
    element
        .get_attribute(attr)
        .and_then(|value| value.parse().ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{absolute_model_offset, rendered_local_to_model_offset};

    #[test]
    fn rendered_local_offsets_account_for_prefixes() {
        assert_eq!(rendered_local_to_model_offset(0, 1, 3), 0);
        assert_eq!(rendered_local_to_model_offset(1, 1, 3), 0);
        assert_eq!(rendered_local_to_model_offset(2, 1, 3), 1);
        assert_eq!(rendered_local_to_model_offset(9, 1, 3), 3);
    }

    #[test]
    fn absolute_offsets_are_clamped_to_block_length() {
        assert_eq!(absolute_model_offset(6, 3, 0, 5, 20), 9);
        assert_eq!(absolute_model_offset(18, 5, 0, 10, 20), 20);
    }
}
