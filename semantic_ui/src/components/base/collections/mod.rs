pub mod collection_create;
pub mod collection_form;
mod collection_item_loader;
mod collection_picker;
pub mod collection_update;

mod entity_collection_manager;
pub use entity_collection_manager::EntityCollectionManager;

use brass::{
    vdom::{self, Func, Render, Renderer},
    Shared, VNode,
};
use factordb::schema::AttributeDescriptor;
use semantic_ui_core::components::Loader;
use semantics_core::base::{AttrCollectionItem, Collection, CollectionWithItems};

use self::collection_update::CollectionUpdate;

pub fn collection_items_loader(
    col: &Collection,
    render: Renderer<Shared<CollectionWithItems>>,
) -> VNode {
    Loader::<Collection, CollectionWithItems> {
        input: col.clone(),
        load: Func::Static(|col| {
            Box::pin(async move {
                let query = CollectionWithItems::build_query(&col);
                let page = crate::api().select(query).await?;
                let col = CollectionWithItems::from_query_result(col, page.items);
                Ok(col)
            })
        }),
        render,
    }
    .render()
}

pub fn collection_content(
    item: &factordb::query::select::Item,
    opts: &semantic_ui_core::EntityRenderOpts,
) -> brass::VNode {
    if opts.preview {
        let item_count = item
            .data
            .get(AttrCollectionItem::QUALIFIED_NAME)
            .and_then(|val| val.as_list())
            .map(|l| l.len().to_string())
            .unwrap_or("?".to_string());
        vdom::p_with(format!("Collection with {} item(s).", item_count)).build()
    } else {
        if opts.editable {
            CollectionUpdate { item: item.clone() }.render()
        } else {
            CollectionUpdate { item: item.clone() }.render()
        }
    }
}
