mod collection_select;

use std::rc::Rc;

use brass::{
    dom::{builder::div, Attr, Render, Tag, TagBuilder, View},
    signal::{
        signal::{Mutable, SignalExt},
        signal_vec::MutableVec,
    },
};
pub use collection_select::CollectionSelect;

mod entity_collection_manager;
pub use entity_collection_manager::entity_collection_manager;

mod collection_form;
use collection_form::collection_metadata_form;

mod collection_item_tagger;

mod collection_item_manager;

use factordb::{
    query::{
        mutate::Mutate,
        select::{Item, Page},
    },
    schema::EntityContainer,
    AnyError, Id,
};
use semantic_core::base::Collection;

use crate::{
    base::collection::collection_item_manager::CollectionItemManager,
    components::{
        entity::entity_list,
        form::FormLoadFuture,
        loader::load,
        util::{modal::modal, notification_warning, ButtonBuilder},
    },
    context::{self, api, router},
    routing::Route,
    EntityRenderOpts,
};

pub fn collection_create(on_created: impl Fn(Collection) + 'static) -> TagBuilder {
    let on_created = Rc::new(on_created);

    let submit = move |col: Collection| -> FormLoadFuture {
        let on_created = on_created.clone();
        Box::pin(async move {
            tracing::trace!("CREATING ENTITY");
            api().entity_create(col.clone()).await?;
            on_created(col);
            Ok(())
        })
    };

    collection_metadata_form(
        Collection {
            id: Id::random(),
            ident: None,
            url: None,
            title: String::new(),
            description: None,
            item_ids: Vec::new(),
            extra: Default::default(),
        },
        submit,
    )
}

fn collection_meta(col: &Collection) -> TagBuilder {
    let mut content = div();

    if let Some(v) = &col.description {
        content.add_child(Tag::P.new().and(v));
    }

    if let Some(url) = &col.url {
        let url = url.to_string();
        content.add_child(
            Tag::P
                .new()
                .and(Tag::A.new().and(&url).attr(Attr::Href, url)),
        );
    }

    content
}

fn collection_meta_edit(col: Collection, on_saved: impl Fn(Collection) + 'static) -> TagBuilder {
    let on_saved = Rc::new(on_saved);
    collection_metadata_form(col, move |col| {
        let on_saved = on_saved.clone();
        Box::pin(async move {
            api()
                .mutate(Mutate::merge(col.id, col.clone().into_map().unwrap()))
                .await?;
            on_saved(col);
            Ok(())
        })
    })
}

pub fn collection_view(
    col: Collection,
    items: Option<&[Item]>,
    opts: &EntityRenderOpts,
) -> TagBuilder {
    if opts.preview {
        if let Some(items) = items {
            return entity_list(
                items,
                &context::registry(),
                &EntityRenderOpts {
                    editable: false,
                    preview: true,
                },
            );
        } else {
            return div().and(format!("Collection with {} items.", col.item_ids.len()));
        }
    }

    let meta = if opts.editable {
        let editing = Mutable::new(false);
        let col = col.clone();

        let meta_form = editing.signal().map(move |is_editing| {
            if is_editing {
                let editing = editing.clone();
                collection_meta_edit(col.clone(), move |_col| {
                    editing.set(false);
                })
            } else {
                let editing = editing.clone();
                div().and(
                    collection_meta(&col).and(
                        ButtonBuilder::new()
                            .label("Edit")
                            .on(move || editing.set(true))
                            .build(),
                    ),
                )
            }
        });

        div().signal(meta_form)
    } else {
        collection_meta(&col)
    };

    let items = load(load_collection_items(col.clone()), move |page| {
        let item_tagging = Mutable::new(false);

        let items2 = page.items.clone();
        let item_tagger = item_tagging.clone().signal_ref(move |flag| -> View {
            if *flag {
                let toggle = item_tagging.clone();
                let content = collection_item_tagger::CollectionItemTagger {
                    items: items2.clone(),
                    on_complete: Rc::new(move || {
                        toggle.set(false);
                    }),
                }
                .render();

                let toggle = item_tagging.clone();
                modal(
                    content,
                    move || {
                        toggle.set(false);
                    },
                    true,
                )
                .into()
            } else {
                let toggle = item_tagging.clone();
                div()
                    .class("mb-2")
                    .and(
                        ButtonBuilder::new()
                            .label("Tag Items")
                            .on(move || {
                                toggle.set(true);
                            })
                            .build(),
                    )
                    .into()
            }
        });

        let manager = CollectionItemManager {
            collection_id: col.id,
            items: MutableVec::new_with_values(page.items.clone()),
        }
        .render();

        div().signal(item_tagger).and(manager).into()
    });

    div().and(meta).and(Tag::Hr.new()).and(items)
}

pub fn collection_content(item: &Item, opts: &EntityRenderOpts) -> TagBuilder {
    if let Ok(col) = Collection::try_from_map(item.data.clone()) {
        let items = item
            .joins
            .iter()
            .find(|j| j.name == Collection::ITEMS_JOIN)
            .map(|j| j.items.as_slice());
        collection_view(col, items, opts)
    } else {
        notification_warning().and("Item is not a collection")
    }
}

pub fn collection_create_page(_item: &Item, _opts: &EntityRenderOpts) -> TagBuilder {
    collection_create(|col| {
        router().goto(Route::Entity(col.id.into()));
    })
}

async fn search_collections(term: String) -> Result<Page<Collection>, AnyError> {
    context::api()
        .select_entities(Collection::search_collections(term, 10))
        .await
}

async fn load_entity_collections(id: Id) -> Result<Page<Collection>, AnyError> {
    context::api()
        .select_entities(Collection::query_collections_with_entity(id))
        .await
}

async fn collection_add_entity(collection: Id, entity: Id) -> Result<(), AnyError> {
    context::api()
        .mutate(Collection::mutate_add_item(collection, entity))
        .await
}

async fn collection_remove_entity(collection: Id, entity: Id) -> Result<(), AnyError> {
    context::api()
        .mutate(Collection::mutate_remove_item(collection, entity))
        .await
}

async fn load_collection_items(col: Collection) -> Result<Page<Item>, AnyError> {
    api().select(Collection::query_collection_items(&col)).await
}
