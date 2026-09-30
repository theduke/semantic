use super::schema::*;
use super::*;
use crate::{
    content::{ATTR_MAIN_CONTENT, MainContent},
    directory_query::{sql_ident, sql_string},
    domain_support::{error, new_id, persist, read, upsert},
    labels::LabelStore,
};
use semantic_data::{
    attr::{ATTR_RELATION_FROM, ATTR_RELATION_RELATION, ATTR_RELATION_TO, ATTR_UPDATED_AT},
    builtin::DEFAULT_COLLECTION,
    query::Batch,
    value::{DateTime, IntoValue, Object},
};
use semantic_rpc_core::RpcError;
static WRITES: futures::lock::Mutex<()> = futures::lock::Mutex::new(());
async fn links(store: &impl LabelStore, target: &EntityTarget) -> Result<Vec<Object>, RpcError> {
    store
        .select(format!(
            "SELECT * FROM {} WHERE type = {} AND {} = {} AND {} = {}",
            sql_ident(DEFAULT_COLLECTION),
            sql_string(RELATION_ID),
            sql_ident(ATTR_RELATION_FROM),
            sql_string(&target.id),
            sql_ident(ATTR_ENTITY_COLLECTION),
            sql_string(&target.collection)
        ))
        .await
}
pub async fn list_comments(
    store: &impl LabelStore,
    p: ListCommentsPayload,
) -> Result<CommentPage, RpcError> {
    let attachments = links(store, &p.target).await?;
    let mut comments = Vec::new();
    for link in attachments {
        let id: String = read(&link, ATTR_RELATION_TO)?;
        if let Some(object) = store.get(DEFAULT_COLLECTION, &id).await? {
            comments.push(Comment::from_object(&object)?);
        }
    }
    comments.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    comments.dedup_by(|a, b| a.id == b.id);
    let total = comments.len() as u64;
    let comments = comments
        .into_iter()
        .skip(p.offset as usize)
        .take(p.limit.clamp(1, 100) as usize)
        .collect();
    Ok(CommentPage { comments, total })
}
pub async fn create_comment(
    store: &impl LabelStore,
    p: CreateCommentPayload,
    author: String,
) -> Result<Comment, RpcError> {
    let _guard = WRITES.lock().await;
    if p.target.collection.is_empty() || p.target.id.is_empty() {
        return Err(error("A comment needs a valid subject"));
    }
    if store
        .get(&p.target.collection, &p.target.id)
        .await?
        .is_none()
    {
        return Err(error("Comment subject not found"));
    }
    p.main_content.validate()?;
    if p.main_content.body().trim().is_empty() {
        return Err(error("Comment cannot be empty"));
    }
    if author.is_empty() {
        return Err(error("Comment author unavailable"));
    }
    if let Some(parent) = &p.parent {
        let object = store
            .get(DEFAULT_COLLECTION, parent)
            .await?
            .ok_or_else(|| error("Reply parent not found"))?;
        let comment = Comment::from_object(&object)?;
        if comment.deleted {
            return Err(error("Cannot reply to a deleted comment"));
        }
        let attachments = links(store, &p.target).await?;
        let ids = attachments
            .iter()
            .map(|o| read::<String>(o, ATTR_RELATION_TO))
            .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
        if !ids.contains(parent) {
            return Err(error("Reply parent belongs to another subject"));
        }
        let mut visited = std::collections::BTreeSet::new();
        let mut cursor = Some(parent.clone());
        while let Some(id) = cursor {
            if !visited.insert(id.clone()) || !ids.contains(&id) {
                return Err(error("Malformed reply hierarchy"));
            }
            let object = store
                .get(DEFAULT_COLLECTION, &id)
                .await?
                .ok_or_else(|| error("Reply ancestor not found"))?;
            cursor = Comment::from_object(&object)?.parent;
        }
    }
    let now = DateTime::now_utc();
    let comment = Comment {
        id: new_id("comment"),
        main_content: p.main_content,
        author_id: author,
        parent: p.parent,
        created_at: now,
        updated_at: now,
        deleted: false,
    };
    let link_id = new_id("comment-link");
    let mut relation = Object::new();
    for (key, value) in [
        ("id", link_id.clone()),
        ("type", RELATION_ID.into()),
        (ATTR_RELATION_RELATION, RELATION_ID.into()),
        (ATTR_RELATION_FROM, p.target.id),
        (ATTR_RELATION_TO, comment.id.clone()),
        (ATTR_ENTITY_COLLECTION, p.target.collection),
    ] {
        relation.insert(key, value.into_value());
    }
    let mut batch = Batch::new();
    batch
        .operations
        .push(upsert(comment.id.clone(), comment.to_object()));
    batch.operations.push(upsert(link_id, relation));
    store.commit(batch).await?;
    Ok(comment)
}
pub async fn edit_comment(
    store: &impl LabelStore,
    id: &str,
    content: Option<MainContent>,
    author: &str,
    privileged: bool,
) -> Result<Comment, RpcError> {
    let _guard = WRITES.lock().await;
    let mut object = store
        .get(DEFAULT_COLLECTION, id)
        .await?
        .ok_or_else(|| error("Comment not found"))?;
    let mut comment = Comment::from_object(&object)?;
    if comment.author_id != author && !privileged {
        return Err(RpcError::new(
            "forbidden",
            "Only the author can change this comment",
        ));
    }
    let now = DateTime::now_utc();
    match content {
        Some(content) => {
            if comment.deleted {
                return Err(error("Deleted comments cannot be edited"));
            }
            content.validate()?;
            if content.body().trim().is_empty() {
                return Err(error("Comment cannot be empty"));
            }
            comment.main_content = content;
            object.insert(ATTR_EDITED_AT, now.into_value());
        }
        None => {
            if comment.deleted {
                return Ok(comment);
            }
            comment.deleted = true;
            comment.main_content = MainContent::text("");
            object.insert(ATTR_DELETED, true.into_value());
            object.insert(ATTR_DELETED_AT, now.into_value());
        }
    }
    comment.updated_at = now;
    object.insert(ATTR_MAIN_CONTENT, comment.main_content.clone().into_value());
    object.insert(ATTR_UPDATED_AT, now.into_value());
    persist(store, id.into(), object).await?;
    Ok(comment)
}
