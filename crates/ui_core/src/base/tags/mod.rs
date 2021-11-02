mod tag_form;
use factordb::{AnyError, Id, query::{
        expr::Expr,
        select::{Item, Select},
    }, schema::{builtin::AttrId, AttrMapExt}};
use semantic_core::base::{AttrTags, Tag};
pub use tag_form::tag_form;

mod tag_create;
pub use tag_create::tag_create;

mod tag_manager;
pub use tag_manager::tag_manager;

mod tag_select;
pub use tag_select::TagSelect;

mod entity_tag_manager;
pub use entity_tag_manager::entity_tag_manager;

pub async fn load_entity_tags(id: Id) -> Result<Vec<Tag>, AnyError> {
    let entity = crate::context::api().entity(id).await?;
    let tag_ids = entity.get_attr_vec::<AttrTags>();

    let filter = Expr::in_(Expr::attr::<AttrId>(), tag_ids);
    let page = crate::context::api()
        .select(Select::new().with_filter(filter).with_limit(1000))
        .await?
        .convert_data::<Tag>()?;
    Ok(page.items)
}

async fn entity_set_tags(entity_id: Id, tags: Vec<Id>) -> Result<(), AnyError> {
    crate::context::api()
        .mutate(Tag::mutate_set_tags(entity_id, tags))
        .await?;
    Ok(())
}

async fn entity_add_tag(entity: Id, tag: Id) -> Result<(), AnyError> {
    crate::context::api()
        .mutate(Tag::mutate_add_tag(entity, tag))
        .await?;
    Ok(())
}

async fn entity_remove_tag(entity: Id, tag: Id) -> Result<(), AnyError> {
    crate::context::api()
        .mutate(Tag::mutate_remove_tag(entity, tag))
        .await?;
    Ok(())
}
