use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, Mutex},
};

use dioxus::prelude::*;
use dxeditor::{
    ActionSurface, BlockNode, ChromeRendererContext, CodecRegistry, ComponentRenderKind,
    ComponentRendererContext, DecodeContext, EditorCatalog, EditorCodec, EditorDocument,
    EditorError, EditorPayload, EditorSelection, EncodeContext, InlineNode, KeyBinding, Mark,
    NodeContent, Operation, TextPosition, Transaction,
};
use futures::executor::block_on;
use serde_json::{Value, json};

#[test]
fn codec_registration_decodes_default_formats() {
    let catalog = EditorCatalog::default();
    let formats = catalog.codecs().formats().collect::<Vec<_>>();

    assert!(formats.contains(&"dxeditor.document.v1"));
    assert!(formats.contains(&"plain_text"));
    assert!(formats.contains(&"markdown"));

    let document = catalog
        .codecs()
        .decode(&EditorPayload::new(
            "plain_text",
            Value::String("hello".into()),
        ))
        .unwrap();
    assert_eq!(document.text_content(), "hello");
}

#[test]
fn custom_codec_can_be_registered() {
    struct CustomCodec;

    impl EditorCodec for CustomCodec {
        fn format(&self) -> &str {
            "custom"
        }

        fn decode(
            &self,
            payload: &EditorPayload,
            _ctx: DecodeContext,
        ) -> Result<EditorDocument, EditorError> {
            Ok(EditorDocument::plain_text(
                payload.value.as_str().unwrap_or_default(),
            ))
        }

        fn encode(
            &self,
            document: &EditorDocument,
            _ctx: EncodeContext,
        ) -> Result<EditorPayload, EditorError> {
            Ok(EditorPayload::new(
                "custom",
                Value::String(document.text_content()),
            ))
        }
    }

    let mut registry = CodecRegistry::default();
    registry.register(Arc::new(CustomCodec));

    let document = registry
        .decode(&EditorPayload::new("custom", Value::String("body".into())))
        .unwrap();
    assert_eq!(document.text_content(), "body");
}

#[test]
fn renderer_overrides_and_fallback_are_lookupable() {
    let mut catalog = EditorCatalog::default();

    catalog.renderers_mut().register_component_renderer(
        "custom",
        Rc::new(|ctx: ComponentRendererContext| {
            let text = ctx
                .block
                .map(|block| block.text_content())
                .unwrap_or_default();
            rsx! { span { "{text}" } }
        }),
    );
    catalog.renderers_mut().register_chrome_renderer(
        "toolbar",
        Rc::new(|_ctx: ChromeRendererContext| rsx! { nav {} }),
    );

    assert!(
        catalog
            .renderers()
            .exact_component_renderer("custom")
            .is_some()
    );
    assert!(catalog.renderers().component_renderer("missing").is_some());
    assert!(catalog.renderers().chrome_renderer("toolbar").is_some());
}

#[test]
fn actions_are_listed_by_surface_and_predicates_run() {
    let catalog = EditorCatalog::default();
    let state = dxeditor::EditorState::new(EditorDocument::plain_text("body"));
    let slash_actions = catalog
        .actions()
        .actions_for_surface(ActionSurface::SlashMenu);

    assert!(
        slash_actions
            .iter()
            .any(|action| action.id == "editor.paragraph")
    );
    assert!(slash_actions.iter().all(|action| action.enabled(&state)));
    assert!(!slash_actions[0].active(&state));
}

#[test]
fn keyboard_dispatch_runs_matching_action() {
    let catalog = EditorCatalog::default();
    let state = dxeditor::EditorState::new(EditorDocument::plain_text("body"));
    let handled = catalog
        .actions()
        .dispatch_key(&KeyBinding::primary("b"), catalog.clone(), state)
        .unwrap();

    assert!(handled);
}

#[test]
fn slash_menu_provider_lists_registered_actions() {
    let catalog = EditorCatalog::default();
    let provider = catalog.suggestions().provider('/').unwrap();
    let items = block_on(provider.query(dxeditor::SuggestionQueryContext {
        query: "heading".to_string(),
        state: dxeditor::EditorState::new(EditorDocument::plain_text("")),
        catalog,
    }));

    assert!(items.iter().any(|item| item.label == "Heading 1"));
}

#[test]
fn command_to_transaction_flow_updates_state_and_history() {
    let catalog = EditorCatalog::default();
    let state = dxeditor::EditorState::new(EditorDocument::plain_text("before"));
    let transaction = catalog
        .commands()
        .dispatch(
            "editor.set_plain_text",
            &state,
            Value::String("after".into()),
        )
        .unwrap();

    state.apply_transaction(transaction).unwrap();

    assert_eq!(state.document().text_content(), "after");
    assert_eq!(state.history().undo_len(), 1);
}

#[test]
fn transaction_normalizes_empty_documents() {
    let state = dxeditor::EditorState::new(EditorDocument::plain_text("before"));
    state
        .apply_transaction(Transaction::new(vec![Operation::ReplaceDocument(
            EditorDocument::new(Vec::new()),
        )]))
        .unwrap();

    assert_eq!(state.document().blocks.len(), 1);
}

#[test]
fn markdown_round_trip_preserves_basic_blocks() {
    let catalog = EditorCatalog::default();
    let input = "# Title\n\nParagraph";
    let document = catalog
        .codecs()
        .decode(&EditorPayload::new("markdown", Value::String(input.into())))
        .unwrap();
    let output = catalog.codecs().encode(&document, "markdown").unwrap();

    assert_eq!(output.value.as_str().unwrap(), input);
}

#[test]
fn markdown_round_trip_preserves_inline_marks_and_links() {
    let catalog = EditorCatalog::default();
    let input =
        "A *soft* **strong** `code` [site](https://example.com) [@Ada](semantic:entity:entity-1)";
    let document = catalog
        .codecs()
        .decode(&EditorPayload::new("markdown", Value::String(input.into())))
        .unwrap();

    let inline = match &document.blocks[0].content {
        NodeContent::Inline(inline) => inline,
        _ => panic!("expected inline content"),
    };

    assert!(inline.iter().any(|node| {
        node.text == "soft" && node.marks.iter().any(|mark| mark.component == "italic")
    }));
    assert!(inline.iter().any(|node| {
        node.text == "strong" && node.marks.iter().any(|mark| mark.component == "bold")
    }));
    assert!(
        inline
            .iter()
            .any(|node| node.text == "code"
                && node.marks.iter().any(|mark| mark.component == "code"))
    );
    assert!(inline.iter().any(|node| {
        node.text == "site"
            && node.marks.iter().any(|mark| {
                mark.component == "link"
                    && mark.attrs.get("href").and_then(Value::as_str) == Some("https://example.com")
            })
    }));
    assert!(
        inline
            .iter()
            .any(|node| node.component == "mention" && node.text == "Ada")
    );

    let output = catalog.codecs().encode(&document, "markdown").unwrap();

    assert_eq!(output.value.as_str().unwrap(), input);
}

#[test]
fn semantic_mentions_serialize_to_markdown_links() {
    let document = EditorDocument::new(vec![BlockNode::paragraph(
        "block-1",
        vec![
            InlineNode::text("text-1", "See "),
            InlineNode::mention("mention-1", "entity-1", "Ada"),
        ],
    )]);
    let catalog = EditorCatalog::default();

    let output = catalog.codecs().encode(&document, "markdown").unwrap();

    assert_eq!(
        output.value.as_str().unwrap(),
        "See [@Ada](semantic:entity:entity-1)"
    );
}

#[test]
fn set_block_type_preserves_inline_content() {
    let catalog = EditorCatalog::default();
    let state = dxeditor::EditorState::new(EditorDocument::new(vec![BlockNode::paragraph(
        "block-1",
        vec![
            InlineNode::text("text-1", "Hello "),
            InlineNode::text("text-2", "world").with_mark(Mark::new("bold")),
        ],
    )]));

    let transaction = catalog
        .commands()
        .dispatch(
            "editor.set_block_type",
            &state,
            json!({ "id": "block-1", "component": "heading", "attrs": { "level": 2 } }),
        )
        .unwrap();
    state.apply_transaction(transaction).unwrap();
    let document = state.document();

    assert_eq!(document.blocks[0].component, "heading");
    assert_eq!(
        document.blocks[0]
            .attrs
            .get("level")
            .and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(document.blocks[0].text_content(), "Hello world");
    let NodeContent::Inline(inline) = &document.blocks[0].content else {
        panic!("expected inline content");
    };
    assert!(inline[1].marks.iter().any(|mark| mark.component == "bold"));
}

#[test]
fn toggle_mark_model_operation_updates_range() {
    let state = dxeditor::EditorState::new(EditorDocument::new(vec![BlockNode::paragraph(
        "block-1",
        vec![InlineNode::text("text-1", "Hello world")],
    )]));
    let selection = EditorSelection {
        anchor: TextPosition::new("block-1", None, 6),
        focus: TextPosition::new("block-1", None, 11),
    };

    state
        .apply_transaction(Transaction::new(vec![Operation::ToggleMark {
            selection,
            mark: Mark::new("bold"),
        }]))
        .unwrap();
    let document = state.document();
    let NodeContent::Inline(inline) = &document.blocks[0].content else {
        panic!("expected inline content");
    };

    assert_eq!(inline.len(), 2);
    assert_eq!(inline[0].text, "Hello ");
    assert_eq!(inline[1].text, "world");
    assert!(inline[1].marks.iter().any(|mark| mark.component == "bold"));
}

#[test]
fn toggle_mark_without_selection_noops() {
    let catalog = EditorCatalog::default();
    let state = dxeditor::EditorState::new(EditorDocument::new(vec![BlockNode::paragraph(
        "block-1",
        vec![InlineNode::text("text-1", "Hello world")],
    )]));

    let transaction = catalog
        .commands()
        .dispatch("editor.toggle_mark", &state, json!({ "mark": "bold" }))
        .unwrap();

    assert!(transaction.operations.is_empty());
}

#[test]
fn set_block_inline_content_command_replaces_inline_nodes() {
    let catalog = EditorCatalog::default();
    let state = dxeditor::EditorState::new(EditorDocument::new(vec![BlockNode::paragraph(
        "block-1",
        vec![InlineNode::text("text-1", "old")],
    )]));
    let inline = vec![InlineNode::text("text-2", "hello").with_mark(Mark::new("bold"))];

    let transaction = catalog
        .commands()
        .dispatch(
            "editor.set_block_inline_content",
            &state,
            json!({ "id": "block-1", "inline": inline }),
        )
        .unwrap();
    state.apply_transaction(transaction).unwrap();

    let document = state.document();
    let NodeContent::Inline(inline) = &document.blocks[0].content else {
        panic!("expected inline content");
    };
    assert_eq!(inline[0].text, "hello");
    assert!(inline[0].marks.iter().any(|mark| mark.component == "bold"));
}

#[test]
fn split_and_merge_block_helpers_preserve_inline_content() {
    let state = dxeditor::EditorState::new(EditorDocument::new(vec![BlockNode::paragraph(
        "block-1",
        vec![
            InlineNode::text("text-1", "Hello "),
            InlineNode::text("text-2", "world").with_mark(Mark::new("italic")),
        ],
    )]));

    state
        .apply_transaction(Transaction::new(vec![Operation::SplitBlock {
            id: "block-1".into(),
            offset: 6,
        }]))
        .unwrap();
    assert_eq!(state.document().blocks.len(), 2);
    assert_eq!(state.document().blocks[0].text_content(), "Hello ");
    assert_eq!(state.document().blocks[1].text_content(), "world");

    let second_id = state.document().blocks[1].id.clone();
    state
        .apply_transaction(Transaction::new(vec![Operation::MergeBlocks {
            first_id: "block-1".into(),
            second_id,
        }]))
        .unwrap();
    let document = state.document();

    assert_eq!(document.blocks.len(), 1);
    assert_eq!(document.blocks[0].text_content(), "Hello world");
    let NodeContent::Inline(inline) = &document.blocks[0].content else {
        panic!("expected inline content");
    };
    assert!(inline.iter().any(
        |node| node.text == "world" && node.marks.iter().any(|mark| mark.component == "italic")
    ));
}

#[test]
fn document_codec_round_trips_json_payload() {
    let catalog = EditorCatalog::default();
    let document = EditorDocument::new(vec![BlockNode::new(
        "block-1",
        "custom",
        serde_json::Map::new(),
        NodeContent::Custom(json!({ "x": 1 })),
    )]);

    let payload = catalog
        .codecs()
        .encode(&document, "dxeditor.document.v1")
        .unwrap();
    let decoded = catalog.codecs().decode(&payload).unwrap();

    assert_eq!(decoded, document);
}

#[test]
fn dioxus_smoke_renders_editor() {
    let seen = Rc::new(RefCell::new(false));

    #[derive(Clone)]
    struct TestProps {
        seen: Rc<RefCell<bool>>,
    }

    impl PartialEq for TestProps {
        fn eq(&self, other: &Self) -> bool {
            Rc::ptr_eq(&self.seen, &other.seen)
        }
    }

    fn app(props: TestProps) -> Element {
        *props.seen.borrow_mut() = true;
        rsx! {
            dxeditor::PlainTextEditor {
                value: "body".to_string(),
                on_change: move |_value: String| {},
            }
        }
    }

    let mut dom = VirtualDom::new_with_props(app, TestProps { seen: seen.clone() });
    dom.rebuild_to_vec();

    assert!(*seen.borrow());
}

#[test]
fn dioxus_smoke_renders_marked_spans() {
    let document = EditorDocument::new(vec![BlockNode::paragraph(
        "block-1",
        vec![
            InlineNode::text("text-1", "Bold").with_mark(Mark::new("bold")),
            InlineNode::text("text-2", " and "),
            InlineNode::text("text-3", "code").with_mark(Mark::new("code")),
            InlineNode::text("text-4", " link").with_mark(Mark::link("https://example.com")),
        ],
    )]);

    let html = dioxus_ssr::render_element(rsx! {
        dxeditor::DocumentView {
            document,
            catalog: EditorCatalog::default(),
        }
    });

    assert!(html.contains("<strong>"));
    assert!(html.contains("<code"));
    assert!(html.contains("<a href=\"https://example.com\""));
}

#[test]
fn action_dispatch_mutates_shared_state() {
    let catalog = EditorCatalog::default();
    let state = dxeditor::EditorState::new(EditorDocument::plain_text("body"));

    catalog
        .actions()
        .dispatch("editor.heading_1", catalog.clone(), state.clone())
        .unwrap();

    assert_eq!(state.document().blocks[0].component, "heading");
}

#[test]
fn custom_action_predicates_are_called() {
    let called = Arc::new(Mutex::new(false));
    let called_predicate = called.clone();
    let mut catalog = EditorCatalog::default();
    catalog.actions_mut().register(dxeditor::EditorAction {
        id: "custom".to_string(),
        label: "Custom".to_string(),
        group: None,
        icon: None,
        surfaces: vec![ActionSurface::CommandPalette],
        shortcut: None,
        priority: 1,
        is_enabled: Rc::new(move |_| {
            *called_predicate.lock().unwrap() = true;
            true
        }),
        is_active: Rc::new(|_| false),
        run: Rc::new(|_| Ok(())),
    });

    let action = catalog.actions().action("custom").unwrap();
    assert!(action.enabled(&dxeditor::EditorState::new(EditorDocument::plain_text(""))));
    assert!(*called.lock().unwrap());
}

#[test]
fn component_renderer_context_supports_block_target() {
    let catalog = EditorCatalog::default();
    let renderer = catalog.renderers().component_renderer("paragraph").unwrap();
    let _element = renderer(ComponentRendererContext {
        kind: ComponentRenderKind::Block,
        block: Some(BlockNode::paragraph(
            "block-1",
            vec![InlineNode::text("text-1", "body")],
        )),
        inline: None,
        mark: None,
        state: dxeditor::EditorState::new(EditorDocument::plain_text("body")),
    });
}
