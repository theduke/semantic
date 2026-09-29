use super::*;
use semantic_data::{
    attr::AttrId,
    builtin::DEFAULT_COLLECTION,
    value::{FromValue, IntoValue, Null, SemanticType, Value},
};
use semantic_rpc_core::{
    AttrScopeId, CommandAdapter, DynCommand, RpcCommand, RpcCommandSpec, RpcError,
};
use std::{future::Future, pin::Pin};

/// Resolves the authorized database for each request, honoring the host's scope rules.
pub trait LabelContext: Sync + 'static {
    type Store: LabelStore;
    fn label_store(
        &self,
        scope_id: Option<String>,
    ) -> impl Future<Output = Result<Self::Store, RpcError>> + Send;
}

macro_rules! command {
    ($type:ident, $name:literal, $payload:ty => $output:ty, $run:ident) => {
        pub struct $type;
        impl RpcCommandSpec for $type {
            type Payload = $payload;
            type Output = $output;
            type Error = RpcError;
            const NAME: &'static str = $name;
        }
        impl<Ctx: LabelContext> RpcCommand<Ctx> for $type {
            fn call<'a>(
                &'a self,
                ctx: &'a Ctx,
                payload: $payload,
            ) -> Pin<Box<dyn Future<Output = Result<$output, RpcError>> + Send + 'a>> {
                Box::pin($run(ctx, payload))
            }
        }
    };
}

command!(ListLabels, "semantic.base.labels.list", LabelScopePayload => Vec<Label>, list);
command!(LoadEntityLabels, "semantic.base.labels.load", EntityPayload => Vec<Label>, load);
command!(AddEntityLabels, "semantic.base.labels.add", EntityLabelsPayload => Vec<Label>, add);
command!(
    RemoveEntityLabels,
    "semantic.base.labels.remove",
    EntityLabelsPayload => Vec<Label>,
    remove
);
command!(
    ReplaceEntityLabels,
    "semantic.base.labels.replace",
    EntityLabelsPayload => Vec<Label>,
    replace
);
command!(SaveLabel, "semantic.base.labels.save", SaveLabelPayload => Label, save);
command!(DeleteLabel, "semantic.base.labels.delete", DeleteLabelPayload => Null, delete);

pub fn commands<Ctx, E>() -> Vec<Box<dyn DynCommand<Ctx, E>>>
where
    Ctx: LabelContext,
    E: From<RpcError>,
{
    vec![
        Box::new(CommandAdapter::new(ListLabels)),
        Box::new(CommandAdapter::new(LoadEntityLabels)),
        Box::new(CommandAdapter::new(AddEntityLabels)),
        Box::new(CommandAdapter::new(RemoveEntityLabels)),
        Box::new(CommandAdapter::new(ReplaceEntityLabels)),
        Box::new(CommandAdapter::new(SaveLabel)),
        Box::new(CommandAdapter::new(DeleteLabel)),
    ]
}

#[derive(SemanticType, IntoValue, FromValue, Clone, Debug, Default)]
#[semantic(namespace = "semantic:base:label")]
pub struct LabelScopePayload {
    #[semantic(attr = AttrScopeId)]
    pub scope_id: Option<String>,
}

/// Identifies an entity; `collection` defaults to the default collection.
#[derive(SemanticType, IntoValue, FromValue, Clone, Debug)]
#[semantic(namespace = "semantic:base:label")]
pub struct EntityPayload {
    #[semantic(attr = AttrScopeId)]
    pub scope_id: Option<String>,
    pub collection: Option<String>,
    /// A nonempty entity id.
    #[semantic(attr = AttrId)]
    pub id: String,
}

#[derive(SemanticType, IntoValue, FromValue, Clone, Debug)]
#[semantic(namespace = "semantic:base:label")]
pub struct EntityLabelsPayload {
    #[semantic(attr = AttrScopeId)]
    pub scope_id: Option<String>,
    pub collection: Option<String>,
    /// A nonempty entity id.
    #[semantic(attr = AttrId)]
    pub id: String,
    /// Nonempty label ids.
    pub label_ids: Vec<String>,
}

#[derive(SemanticType, IntoValue, FromValue, Clone, Debug)]
#[semantic(namespace = "semantic:base:label")]
pub struct SaveLabelPayload {
    #[semantic(attr = AttrScopeId)]
    pub scope_id: Option<String>,
    pub label: Label,
}

#[derive(SemanticType, IntoValue, FromValue, Clone, Debug)]
#[semantic(namespace = "semantic:base:label")]
pub struct DeleteLabelPayload {
    #[semantic(attr = AttrScopeId)]
    pub scope_id: Option<String>,
    /// A nonempty label id.
    #[semantic(attr = AttrId)]
    pub id: String,
}

fn non_empty(value: String, key: &str) -> Result<String, RpcError> {
    if value.trim().is_empty() {
        Err(RpcError::invalid_payload(format!("{key} is required")))
    } else {
        Ok(value)
    }
}

fn label_ids(ids: Vec<String>) -> Result<Vec<String>, RpcError> {
    if ids.iter().any(|id| id.trim().is_empty()) {
        return Err(RpcError::invalid_payload(
            "label_ids must contain nonempty strings",
        ));
    }
    Ok(ids)
}

async fn list(ctx: &impl LabelContext, payload: LabelScopePayload) -> Result<Vec<Label>, RpcError> {
    list_labels(&ctx.label_store(payload.scope_id).await?).await
}

async fn load(ctx: &impl LabelContext, payload: EntityPayload) -> Result<Vec<Label>, RpcError> {
    let store = ctx.label_store(payload.scope_id).await?;
    let collection = payload
        .collection
        .unwrap_or_else(|| DEFAULT_COLLECTION.into());
    let id = non_empty(payload.id, "id")?;
    labels_for_entity(&store, &collection, &id).await
}

#[derive(Clone, Copy)]
enum Assignment {
    Add,
    Remove,
    Replace,
}

async fn assign(
    ctx: &impl LabelContext,
    payload: EntityLabelsPayload,
    assignment: Assignment,
) -> Result<Vec<Label>, RpcError> {
    let store = ctx.label_store(payload.scope_id).await?;
    let collection = payload
        .collection
        .unwrap_or_else(|| DEFAULT_COLLECTION.into());
    let id = non_empty(payload.id, "id")?;
    let ids = label_ids(payload.label_ids)?;
    match assignment {
        Assignment::Add => add_labels(&store, &collection, &id, &ids).await,
        Assignment::Remove => remove_labels(&store, &collection, &id, &ids).await,
        Assignment::Replace => replace_labels(&store, &collection, &id, &ids).await,
    }
}

async fn add(
    ctx: &impl LabelContext,
    payload: EntityLabelsPayload,
) -> Result<Vec<Label>, RpcError> {
    assign(ctx, payload, Assignment::Add).await
}

async fn remove(
    ctx: &impl LabelContext,
    payload: EntityLabelsPayload,
) -> Result<Vec<Label>, RpcError> {
    assign(ctx, payload, Assignment::Remove).await
}

async fn replace(
    ctx: &impl LabelContext,
    payload: EntityLabelsPayload,
) -> Result<Vec<Label>, RpcError> {
    assign(ctx, payload, Assignment::Replace).await
}

async fn save(ctx: &impl LabelContext, payload: SaveLabelPayload) -> Result<Label, RpcError> {
    let store = ctx.label_store(payload.scope_id).await?;
    save_label(&store, payload.label).await
}

async fn delete(ctx: &impl LabelContext, payload: DeleteLabelPayload) -> Result<Null, RpcError> {
    let store = ctx.label_store(payload.scope_id).await?;
    delete_label(&store, &non_empty(payload.id, "id")?).await?;
    Ok(Null)
}

pub fn encode_labels(labels: Vec<Label>) -> Value {
    labels.into_value()
}

pub fn decode_labels(value: Value) -> Result<Vec<Label>, RpcError> {
    Vec::<Label>::from_value(value).map_err(|err| RpcError::invalid_output(err.describe("output")))
}
