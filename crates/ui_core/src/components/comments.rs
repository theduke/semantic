use crate::{
    EntityTarget, MainContentEditor, MainContentView, use_active_scope_id, use_rpc_client,
};
use dioxus::prelude::*;
use semantic_base::{
    comments::{
        Comment, CreateComment, CreateCommentPayload, DeleteComment, DeleteCommentPayload,
        EditComment, EditCommentPayload, ListComments, ListCommentsPayload,
    },
    content::MainContent,
};
use semantic_data::value::{FromValue, IntoValue, Object, Value};

#[component]
pub fn CommentComposer(
    on_submit: EventHandler<Value>,
    #[props(default)] initial: Option<Value>,
    #[props(default)] saving: bool,
    #[props(default)] error: Option<String>,
    #[props(default)] on_cancel: Option<EventHandler<()>>,
    #[props(default = "Write a comment".to_string())] label: String,
    #[props(default = "Post comment".to_string())] submit_label: String,
) -> Element {
    let mut draft = use_signal(|| initial.unwrap_or_else(|| MainContent::note("").into_value()));
    let nonblank =
        MainContent::from_value(draft()).is_ok_and(|content| !content.body().trim().is_empty());
    rsx! { div { class: "semantic-comment-composer",
        p { class: "semantic-comment-composer__label", "{label}" }
        MainContentEditor { value: draft(), label, disabled: saving, on_change: move |value| draft.set(value) }
        if let Some(error) = error { p { class: "semantic-comments__error", role: "alert", "{error}" } }
        div { class: "semantic-comment-composer__actions",
            button { r#type: "button", class: "semantic-comments__primary", disabled: saving || !nonblank,
                onclick: move |_| { if !saving && nonblank { on_submit.call(draft()); } },
                if saving { "Saving…" } else { "{submit_label}" }
            }
            if let Some(cancel) = on_cancel { button { r#type: "button", disabled: saving, onclick: move |_| cancel.call(()), "Cancel" } }
        }
    } }
}

/// Changes of scope or collection create a fresh session, including drafts.
#[component]
/// Comments for the active scope and collection-qualified subject. The host
/// must expose `semantic.app.capabilities` with a scoped `comments` boolean in
/// addition to the comments commands. SemanticApp provides this integration;
/// registering CommentsPackage alone does not provide the capability command.
pub fn EntityComments(target: EntityTarget, #[props(default = true)] threaded: bool) -> Element {
    let scope = use_active_scope_id();
    let key = format!("{:?}:{:?}", scope, target);
    rsx! { for (key, target, scope) in [(key, target, scope)] {
        CommentsSession { key: "{key}", target, scope, threaded }
    } }
}

#[component]
fn CommentsSession(target: EntityTarget, scope: Option<String>, threaded: bool) -> Element {
    let client = use_rpc_client();
    let mut revision = use_signal(|| 0_u64);
    let mut limit = use_signal(|| 100_u64);
    let mut saving = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut composer_key = use_signal(|| 0_u64);
    let mut reveal = use_signal(|| None::<String>);
    let mut presentation = use_signal(|| threaded);
    let load_client = client.clone();
    let load_target = target.clone();
    let load_scope = scope.clone();
    let mut page = use_resource(move || {
        let client = load_client.clone();
        let target = load_target.clone();
        let scope = load_scope.clone();
        let limit = limit();
        let _ = revision();
        let reveal = reveal();
        async move {
            let mut payload = Object::new();
            if let Some(scope) = &scope {
                payload.insert("scope_id", Value::String(scope.clone()));
            }
            let capabilities = client
                .invoke_value("semantic.app.capabilities", Value::Object(payload))
                .await
                .map_err(|e| e.to_string())?;
            if !matches!(capabilities, Value::Object(ref obj) if obj.get("comments") == Some(&Value::Bool(true)))
            {
                return Ok::<Option<semantic_base::comments::CommentPage>, String>(None);
            }
            let target = semantic_base::comments::EntityTarget {
                collection: target.collection_or_default().into(),
                id: target.id,
            };
            let mut comments = Vec::new();
            let total = loop {
                let loaded = client
                    .invoke::<ListComments>(ListCommentsPayload {
                        scope_id: scope.clone(),
                        target: target.clone(),
                        offset: comments.len() as u64,
                        limit: 100,
                    })
                    .await
                    .map_err(|e| e.to_string())?;
                let empty = loaded.comments.is_empty();
                comments.extend(loaded.comments);
                if empty
                    || comment_window_complete(&comments, loaded.total, limit, reveal.as_deref())
                {
                    break loaded.total;
                }
            };
            Ok(Some(semantic_base::comments::CommentPage {
                comments,
                total,
            }))
        }
    });
    if matches!(&*page.read(), Some(Ok(None))) {
        return rsx! {};
    }
    let create_client = client.clone();
    let create_target = target.clone();
    let create_scope = scope.clone();
    rsx! { section { class: "semantic-comments", aria_label: "Comments",
        style { {include_str!("comments.css")} }
        match &*page.read() {
            None => rsx! { p { role: "status", "Loading comments…" } },
            Some(Err(message)) => rsx! { div { class: "semantic-comments__error", role: "alert", "{message}", button { r#type: "button", onclick: move |_| page.restart(), "Retry" } } },
            Some(Ok(None)) => rsx! {},
                Some(Ok(Some(data))) => rsx! {
                    if *page.state().read() == UseResourceState::Pending { p { role: "status", "Refreshing comments…" } }
                header { class: "semantic-comments__header",
                    h3 { "Comments" span { class: "semantic-comments__count", "{data.total}" } }
                    label { input { r#type: "checkbox", checked: presentation(), onchange: move |event| presentation.set(event.checked()) } "Threaded" }
                }
                if data.comments.is_empty() { p { class: "semantic-comments__empty", "Start the conversation. Add context, ask a question, or share an update." } }
                    CommentTree { comments: data.comments.clone(), target: target.clone(), scope: scope.clone(), threaded: presentation(), on_changed: move |_| revision += 1,
                        on_created: move |comment: Comment| { reveal.set(Some(comment.id)); revision += 1; }
                    }
                    if (data.comments.len() as u64) < data.total {
                        button { r#type: "button", disabled: *page.state().read() == UseResourceState::Pending,
                            onclick: { let loaded = data.comments.len() as u64; move |_| limit.set(loaded.saturating_add(100)) }, "Load more comments" }
                }
                for key in [composer_key()] {
                    CommentComposer { key: "{key}", saving: saving(), error: error(),
                        on_submit: {
                            let client = create_client.clone(); let target = create_target.clone(); let scope = create_scope.clone();
                            move |value| {
                                if saving() { return; }
                                let Ok(main_content) = MainContent::from_value(value) else { error.set(Some("Unsupported comment content".into())); return; };
                                saving.set(true); error.set(None);
                                let client = client.clone(); let target = target.clone(); let scope = scope.clone();
                                spawn(async move {
                                    let result = client.invoke::<CreateComment>(CreateCommentPayload {
                                        scope_id: scope, target: semantic_base::comments::EntityTarget { collection: target.collection_or_default().into(), id: target.id }, main_content, parent: None,
                                    }).await;
                                    saving.set(false);
                                match result { Ok(comment) => { reveal.set(Some(comment.id)); composer_key += 1; revision += 1; }, Err(e) => error.set(Some(e.to_string())) }
                                });
                            }
                        }
                    }
                }
            },
        }
    } }
}

/// Load consecutive chronological pages until both the visible window and the
/// newly posted comment are present. This also loads its ancestors for threading.
fn comment_window_complete(
    comments: &[Comment],
    total: u64,
    limit: u64,
    reveal: Option<&str>,
) -> bool {
    comments.len() as u64 >= total
        || (comments.len() as u64 >= limit
            && reveal.is_none_or(|id| comments.iter().any(|comment| comment.id == id)))
}

/// A bounded walk over parent links handles orphaned and cyclic historical rows.
pub fn comment_depths(comments: &[Comment], threaded: bool) -> Vec<(Comment, usize)> {
    let ids: std::collections::HashMap<&str, &Comment> =
        comments.iter().map(|row| (row.id.as_str(), row)).collect();
    let mut rows: Vec<(Comment, usize, Vec<usize>)> = comments
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let mut seen = std::collections::HashSet::from([row.id.as_str()]);
            let mut cursor = row.parent.as_deref();
            let mut ancestry = vec![index];
            while let Some(parent) = cursor.and_then(|id| ids.get(id).copied()) {
                if !seen.insert(parent.id.as_str()) {
                    break;
                }
                if let Some(position) = comments.iter().position(|item| item.id == parent.id) {
                    ancestry.push(position);
                }
                cursor = parent.parent.as_deref();
            }
            ancestry.reverse();
            (
                row.clone(),
                if threaded {
                    ancestry.len().saturating_sub(1)
                } else {
                    0
                },
                ancestry,
            )
        })
        .collect();
    if threaded {
        rows.sort_by(|a, b| a.2.cmp(&b.2));
    }
    rows.into_iter()
        .map(|(row, depth, _)| (row, depth))
        .collect()
}

#[component]
pub fn CommentTree(
    comments: Vec<Comment>,
    target: EntityTarget,
    scope: Option<String>,
    #[props(default = true)] threaded: bool,
    on_changed: EventHandler<()>,
    #[props(default)] on_created: Option<EventHandler<Comment>>,
) -> Element {
    rsx! { div { class: "semantic-comment-tree",
        for (comment, depth) in comment_depths(&comments, threaded) {
            CommentRow { key: "{comment.id}", comment, depth, target: target.clone(), scope: scope.clone(), on_changed, on_created }
        }
    } }
}

#[component]
fn CommentRow(
    comment: Comment,
    depth: usize,
    target: EntityTarget,
    scope: Option<String>,
    on_changed: EventHandler<()>,
    on_created: Option<EventHandler<Comment>>,
) -> Element {
    let client = use_rpc_client();
    let mut action = use_signal(|| None::<&'static str>);
    let mut saving = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let reply_client = client.clone();
    let reply_scope = scope.clone();
    let reply_id = comment.id.clone();
    let delete_id = comment.id.clone();
    let timestamp: time::OffsetDateTime = comment.created_at.into();
    let date = timestamp
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let display_date = format!(
        "{} {:02}, {} · {:02}:{:02} UTC",
        timestamp.month(),
        timestamp.day(),
        timestamp.year(),
        timestamp.hour(),
        timestamp.minute()
    );
    let edited = comment.updated_at != comment.created_at;
    rsx! { article { class: "semantic-comment", style: "--comment-depth: {depth.min(4)}",
        header { class: "semantic-comment__header",
            strong { "{comment.author_id}" }
            time { datetime: "{date}", "{display_date}" }
            if edited && !comment.deleted { span { "Edited" } }
        }
        if depth > 4 { small { "Nested reply" } }
        if comment.parent.is_some() && depth == 0 { small { "Reply" } }
        if comment.deleted { p { class: "semantic-comment__tombstone", "This comment was deleted." } }
        else { MainContentView { value: comment.main_content.clone().into_value() } }
        if !comment.deleted {
            div { class: "semantic-comment__actions",
                button { r#type: "button", disabled: saving(), onclick: move |_| { action.set(Some("reply")); error.set(None); }, "Reply" }
                button { r#type: "button", disabled: saving(), onclick: move |_| { action.set(Some("edit")); error.set(None); }, "Edit" }
                button { r#type: "button", disabled: saving(), onclick: {
                    let client = client.clone(); let scope = scope.clone();
                    move |_| {
                        if saving() { return; } saving.set(true); error.set(None);
                        let client = client.clone(); let scope = scope.clone(); let id = delete_id.clone();
                        spawn(async move { let result = client.invoke::<DeleteComment>(DeleteCommentPayload { scope_id: scope, id }).await;
                            saving.set(false); match result { Ok(_) => on_changed.call(()), Err(e) => error.set(Some(e.to_string())) }
                        });
                    }
                }, "Delete" }
            }
        }
        if action().is_none() { if let Some(message) = error() { p { role: "alert", class: "semantic-comments__error", "{message}" } } }
        if let Some(mode) = action() {
            CommentComposer { key: "{mode}", initial: if mode == "edit" { Some(comment.main_content.clone().into_value()) } else { None },
                saving: saving(), error: error(), label: if mode == "edit" { "Edit comment" } else { "Write a reply" }, submit_label: if mode == "edit" { "Save changes" } else { "Post reply" },
                on_cancel: move |_| { action.set(None); error.set(None); },
                on_submit: {
                    let client = reply_client.clone(); let scope = reply_scope.clone(); let id = reply_id.clone(); let target = target.clone();
                    move |value| {
                        if saving() { return; }
                        let Ok(main_content) = MainContent::from_value(value) else { return; }; saving.set(true); error.set(None);
                        let client = client.clone(); let scope = scope.clone(); let id = id.clone(); let target = target.clone();
                        spawn(async move {
                            let result = if mode == "edit" {
                                    client.invoke::<EditComment>(EditCommentPayload { scope_id: scope, id, main_content }).await
                            } else {
                                    client.invoke::<CreateComment>(CreateCommentPayload { scope_id: scope, target: semantic_base::comments::EntityTarget { collection: target.collection_or_default().into(), id: target.id }, main_content, parent: Some(id) }).await
                            };
                                saving.set(false); match result { Ok(comment) => { action.set(None); if mode == "reply" { if let Some(on_created) = on_created { on_created.call(comment); } else { on_changed.call(()); } } else { on_changed.call(()); } }, Err(e) => error.set(Some(e.to_string())) }
                        });
                    }
                }
            }
        }
    } }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn comment(id: &str, parent: Option<&str>) -> Comment {
        let now = semantic_data::value::DateTime::now_utc();
        Comment {
            id: id.into(),
            main_content: MainContent::note(id),
            author_id: "author".into(),
            parent: parent.map(str::to_owned),
            created_at: now,
            updated_at: now,
            deleted: false,
        }
    }
    #[test]
    fn threaded_rows_group_replies_without_changing_flat_order() {
        let rows = vec![
            comment("first", None),
            comment("second", None),
            comment("reply", Some("first")),
            comment("nested", Some("reply")),
        ];
        let tree = comment_depths(&rows, true);
        assert_eq!(
            tree.iter()
                .map(|(row, depth)| (row.id.as_str(), *depth))
                .collect::<Vec<_>>(),
            vec![("first", 0), ("reply", 1), ("nested", 2), ("second", 0)]
        );
        assert_eq!(
            comment_depths(&rows, false)
                .iter()
                .map(|(row, depth)| (row.id.as_str(), *depth))
                .collect::<Vec<_>>(),
            vec![("first", 0), ("second", 0), ("reply", 0), ("nested", 0)]
        );
    }
    #[test]
    fn historical_cycles_and_orphans_render_once_without_recursion() {
        let rows = vec![
            comment("a", Some("b")),
            comment("b", Some("a")),
            comment("orphan", Some("missing")),
        ];
        let tree = comment_depths(&rows, true);
        assert_eq!(tree.len(), rows.len());
        let ids: std::collections::HashSet<_> =
            tree.iter().map(|(row, _)| row.id.as_str()).collect();
        assert_eq!(ids.len(), rows.len());
        assert!(tree.iter().all(|(_, depth)| *depth < rows.len()));
    }

    #[test]
    fn posting_past_first_page_reveals_root_and_reply_in_thread_order() {
        let mut comments: Vec<_> = (0..100)
            .map(|index| comment(&format!("old-{index}"), None))
            .collect();
        assert!(comment_window_complete(&comments, 102, 100, None));
        assert!(!comment_window_complete(
            &comments,
            102,
            100,
            Some("posted-root")
        ));
        assert!(!comment_window_complete(
            &comments,
            102,
            100,
            Some("posted-reply")
        ));
        comments.push(comment("posted-root", None));
        assert!(comment_window_complete(
            &comments,
            102,
            100,
            Some("posted-root")
        ));
        assert!(!comment_window_complete(
            &comments,
            102,
            100,
            Some("posted-reply")
        ));
        comments.push(comment("posted-reply", Some("old-0")));
        assert!(comment_window_complete(
            &comments,
            102,
            100,
            Some("posted-reply")
        ));
        let rows = comment_depths(&comments, true);
        assert_eq!(rows.len(), 102);
        assert_eq!(rows[0].0.id, "old-0");
        assert_eq!((rows[1].0.id.as_str(), rows[1].1), ("posted-reply", 1));
        assert_eq!(rows.last().unwrap().0.id, "posted-root");
        // A remotely removed reveal target must not cause an endless load.
        assert!(comment_window_complete(
            &comments,
            102,
            200,
            Some("removed")
        ));
    }
}
