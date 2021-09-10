use std::collections::HashSet;

use brass::vdom;
use factordb::{
    data::value::from_value_map,
    query::{
        expr::Expr,
        select::{Item, Page, Select},
    },
    schema::{
        builtin::{AttrId, AttrType},
        AttrMapExt, AttributeDescriptor, EntityDescriptor,
    },
    AnyError, Id,
};
use semantic_core::base::{AttrTagName, AttrTags, Tag};
use semantic_ui_core::{
    components::small_title,
    loader::{error_msg, LoadState},
    ContextExt,
};

use crate::components::entity::entity_search_autocomplete::EntitySearchAutocomplete;

/// Allows managing the collections an entity is part of.
// TODO: this component is very similar to EntityCollectionManager.
// Should be possible to partially unify these with a shared abstraction.
pub struct EntityTagManager {
    pub entity_id: Id,
}

enum Msg {
    CurrentLoaded(Result<Page<Tag>, AnyError>),
    Remove(Tag),
    RemoveLoaded(Result<Tag, AnyError>),
    Add(Item),
    AddLoaded(Result<Tag, AnyError>),
}

struct State {
    current_tags_loader: LoadState<Page<Tag>>,
    remove_loader: LoadState<()>,
    add_loader: LoadState<()>,
}

brass::enable_props!(wrapped EntityTagManager => State);

impl brass::PropComponent for State {
    type Properties = EntityTagManager;
    type Msg = Msg;

    fn init(props: &Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        let entity_id = props.entity_id;
        let api = ctx.api().clone();
        let guard = ctx.run_map(
            async move {
                let entity = api.entity(entity_id).await?;
                let tag_ids = entity.get_attr_vec::<AttrTags>();

                let filter = Expr::in_(Expr::attr::<AttrId>(), tag_ids);
                let page = api
                    .select(Select::new().with_filter(filter).with_limit(1000))
                    .await?
                    .convert_data::<Tag>()?;
                Ok(page)
            },
            Msg::CurrentLoaded,
        );

        Self {
            current_tags_loader: LoadState::Loading(Some(guard)),
            remove_loader: LoadState::Idle,
            add_loader: LoadState::Idle,
        }
    }

    fn update(
        &mut self,
        msg: Self::Msg,
        props: &Self::Properties,
        ctx: &mut brass::Context<Self::Msg>,
    ) {
        match msg {
            Msg::CurrentLoaded(res) => {
                self.current_tags_loader.set_result(res);
            }
            Msg::Remove(tag) => {
                let entity_id = props.entity_id;

                if let Some(current_tags) = self.current_tags_loader.as_success() {
                    let current_tag_ids: Vec<_> = current_tags.items.iter().map(|t| t.id).collect();

                    let api = ctx.api().clone();
                    let guard = ctx.run_map(
                        async move {
                            let mutate =
                                Tag::mutate_remove_tag(entity_id, &current_tag_ids, tag.id);
                            api.mutate(mutate).await?;
                            Ok(tag)
                        },
                        Msg::RemoveLoaded,
                    );
                    self.remove_loader.set_loading_guarded(guard);
                }
            }
            Msg::RemoveLoaded(res) => match res {
                Ok(col) => {
                    self.current_tags_loader
                        .as_success_mut()
                        .map(|page| page.items.retain(|item| item.id != col.id));
                    self.remove_loader.set_idle();
                }
                Err(err) => {
                    self.remove_loader.set_failed(err);
                }
            },
            Msg::Add(item) => {
                if self.add_loader.is_loading() {
                    return;
                }
                let entity_id = props.entity_id;
                let api = ctx.api().clone();
                let guard = ctx.run_map(
                    async move {
                        let tag: Tag = from_value_map(item.data)?;
                        let mutate = Tag::mutate_add_tag(entity_id, tag.id);
                        api.mutate(mutate).await?;
                        Ok(tag)
                    },
                    Msg::AddLoaded,
                );
                self.add_loader.set_loading_guarded(guard);
            }
            Msg::AddLoaded(res) => match res {
                Ok(col) => {
                    if let Some(page) = self.current_tags_loader.as_success_mut() {
                        page.items.push(col);
                    }
                    self.add_loader.set_success(());
                }
                Err(err) => {
                    self.add_loader.set_failed(err);
                }
            },
        }
    }

    fn render(
        &self,
        props: &Self::Properties,
        ctx: &mut brass::RenderContext<brass::PropWrapper<Self>>,
    ) -> brass::VNode {
        self.current_tags_loader.render(|tags| {
            let remove_loading = self.remove_loader.is_loading();

            let current_tags = if tags.items.is_empty() {
                brass_bulma::notification(brass_bulma::Color::Default, "No tags assigned.").build()
            } else {
                let items = tags.items.iter().map(|tag| {
                    let title = tag.name.clone();
                    let tag2 = tag.clone();
                    let toggle = vdom::button()
                        .and_class(brass_bulma::Color::Danger.as_class())
                        .and_class("ml-4")
                        .and(brass_bulma::icon_fa("fas fa-minus-circle"))
                        .style_raw("pl-4")
                        .attr_toggle_if(remove_loading, brass::dom::Attr::Disabled)
                        .on_click(ctx, move || Msg::Remove(tag2.clone()));
                    vdom::div().class("mb-2").and(title).and(toggle)
                });

                vdom::div().and_iter(items).build()
            };

            let remove_err = self
                .remove_loader
                .as_error()
                .map(|err| error_msg(err))
                .unwrap_or_else(|| vdom::div());

            let mut ignored: HashSet<Id> = self
                .current_tags_loader
                .as_success()
                .map(|page| page.items.iter().map(|x| x.id).collect::<HashSet<_>>())
                .unwrap_or_default();
            ignored.insert(props.entity_id);

            let finder = EntitySearchAutocomplete {
                placeholder: Some("Collection title...".into()),
                attribute: Some(AttrTagName::QUALIFIED_NAME.into()),
                filter: Some(Expr::eq(Expr::attr::<AttrType>(), Tag::QUALIFIED_NAME)),
                renderer: Some(vdom::RefRenderer::Static(|item| {
                    let name = item
                        .data
                        .get_attr::<AttrTagName>()
                        .unwrap_or_else(|| "<??>".to_string());
                    vdom::text(name)
                })),
                on_select: ctx.callback_map(Msg::Add),
                ignored_ids: Some(ignored),
            };

            let finder_wrap = vdom::div().and((small_title("Add Tags"), finder));

            vdom::div()
                .and((
                    small_title("Tags"),
                    current_tags,
                    remove_err,
                    vdom::hr(),
                    finder_wrap,
                ))
                .build()
        })
    }
}
