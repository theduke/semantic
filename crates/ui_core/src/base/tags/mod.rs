mod tag_form;
use factdb::{AttrId, AttrMapExt, Expr, Id, Select};
use semantic_core::base::{AttrTags, Tag};
pub use tag_form::tag_form;

mod tag_create;
pub use tag_create::tag_create;

mod tag_manager;
pub use tag_manager::tag_manager;

mod tag_select;
pub use tag_select::{form_field_tags, TagSelect};

mod entity_tag_manager;
pub use entity_tag_manager::entity_tag_manager;

mod tag_merger;
pub use tag_merger::TagMerger;

pub async fn load_entity_tags(id: Id) -> Result<Vec<Tag>, anyhow::Error> {
    let entity = crate::context::api().entity(id).await?;
    let tag_ids = entity.get_attr_vec::<AttrTags>();

    let filter = Expr::in_(Expr::attr::<AttrId>(), tag_ids);
    crate::context::api()
        .select_entities::<Tag>(Select::new().with_filter(filter).with_limit(1000))
        .await
}

pub async fn load_all_tags() -> Result<Vec<Tag>, anyhow::Error> {
    crate::context::api()
        .select_entities(Tag::query_all())
        .await
}

pub async fn entity_set_tags(entity_id: Id, tags: Vec<Id>) -> Result<(), anyhow::Error> {
    crate::context::api()
        .mutate(Tag::mutate_set_tags(entity_id, tags))
        .await?;
    Ok(())
}

async fn entity_add_tag(entity: Id, tag: Id) -> Result<(), anyhow::Error> {
    crate::context::api()
        .mutate(Tag::mutate_add_tag(entity, tag))
        .await?;
    Ok(())
}

async fn entity_remove_tag(entity: Id, tag: Id) -> Result<(), anyhow::Error> {
    crate::context::api()
        .mutate(Tag::mutate_remove_tag(entity, tag))
        .await?;
    Ok(())
}
