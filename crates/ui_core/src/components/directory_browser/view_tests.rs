use dioxus::prelude::*;
use std::{cell::Cell, collections::BTreeSet, rc::Rc};

use super::{DirectoryBrowseItem, DirectoryBrowserCommand, DirectoryGrid, DirectoryList};

// The virtualizer waits for viewport measurements from JavaScript before
// rendering rows. Supply that message when exercising it without a browser.
struct ViewportDocument;

impl document::Document for ViewportDocument {
    fn eval(&self, js: String) -> document::Eval {
        if !js.contains("isScrolling") {
            return document::NoOpDocument.eval(js);
        }
        struct ViewportEval(Option<serde_json::Value>);
        impl document::Evaluator for ViewportEval {
            fn send(&self, _: serde_json::Value) -> Result<(), document::EvalError> {
                Ok(())
            }

            fn poll_recv(
                &mut self,
                _: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Result<serde_json::Value, document::EvalError>> {
                match self.0.take() {
                    Some(value) => std::task::Poll::Ready(Ok(value)),
                    None => std::task::Poll::Pending,
                }
            }

            fn poll_join(
                &mut self,
                _: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Result<serde_json::Value, document::EvalError>> {
                std::task::Poll::Ready(Ok(serde_json::Value::Null))
            }
        }
        document::Eval::new(
            dioxus::core::current_owner::<UnsyncStorage>().insert(Box::new(ViewportEval(Some(
                serde_json::json!({
                    "offset": 0, "viewport": 600, "isScrolling": false
                }),
            )))),
        )
    }
}

#[derive(Clone, PartialEq)]
struct ListingState {
    items: Vec<DirectoryBrowseItem>,
    selected: BTreeSet<String>,
}

#[derive(Clone)]
struct ListingProps {
    icons: bool,
    state: Rc<Cell<Option<Signal<ListingState>>>>,
}

fn listing(props: ListingProps) -> Element {
    let state = use_signal(|| ListingState {
        items: vec![listing_item("parent")],
        selected: BTreeSet::new(),
    });
    use_hook(|| props.state.set(Some(state)));
    let commands = use_coroutine(
        |_: futures::channel::mpsc::UnboundedReceiver<DirectoryBrowserCommand>| async {},
    );
    rsx! {
        if props.icons {
            DirectoryGrid {
                items: state.read().items.clone(),
                selected_items: state.read().selected.clone(),
                cut_items: BTreeSet::new(),
                focused_item: None,
                dragged_item: None,
                commands,
            }
        } else {
            DirectoryList {
                items: state.read().items.clone(),
                selected_items: state.read().selected.clone(),
                cut_items: BTreeSet::new(),
                focused_item: None,
                dragged_item: None,
                commands,
            }
        }
    }
}

fn listing_item(id: &str) -> DirectoryBrowseItem {
    DirectoryBrowseItem {
        id: id.to_string(),
        title: id.to_string(),
        collection: "entities".to_string(),
        object: Default::default(),
        type_id: None,
        is_directory: true,
        has_semantic_children: false,
        order: None,
        created_at: None,
        updated_at: None,
    }
}

#[test]
fn virtual_listings_refresh_items_and_selection_without_count_changes() {
    for icons in [false, true] {
        let handle = Rc::new(Cell::new(None));
        let mut dom = VirtualDom::new_with_props(
            listing,
            ListingProps {
                icons,
                state: handle.clone(),
            },
        );
        dom.insert_any_root_context(Box::new(
            Rc::new(ViewportDocument) as Rc<dyn document::Document>
        ));
        dom.rebuild_to_vec();
        dom.render_immediate_to_vec();
        let mut state = handle.get().unwrap();

        // Navigation and browser history can replace the entire listing
        // without changing how many rows the virtualizer needs.
        for id in ["child", "parent", "child"] {
            state.write().items = vec![listing_item(id)];
            let edits = dom.render_immediate_to_vec().edits;
            assert!(
                edits.iter().any(|edit| matches!(
                    edit,
                    dioxus::core::Mutation::CreateTextNode { value, .. }
                        | dioxus::core::Mutation::SetText { value, .. } if value == id
                )),
                "listing did not update to {id} (icons={icons}): {edits:?}"
            );
        }

        state.write().selected.insert("child".to_string());
        let edits = dom.render_immediate_to_vec().edits;
        assert!(
            edits.iter().any(|edit| matches!(
                edit,
                dioxus::core::Mutation::SetAttribute {
                    name: "data-selected",
                    value: dioxus::core::AttributeValue::Bool(true),
                    ..
                }
            )),
            "selection did not update (icons={icons}): {edits:?}"
        );
    }
}
