use std::collections::HashSet;

use brass::{
    dom::Attr,
    vdom::{self, div_with, event::ClickEvent, s},
    Callback, PropComponent, Shared, VNode,
};
use factordb::{query::select::Item, schema::AttrMapExt};
use semantic_core::base::{Collection, CollectionWithItems};
use semantic_ui_core::{ContextExt, Registry, SharedRegistry};

use crate::components::entity::{
    entity_search_autocomplete::EntitySearchAutocomplete, entity_view::EntityView,
};

pub struct CollectionForm {
    pub item: Shared<CollectionWithItems>,
    pub on_submit: Callback<Collection>,
    pub is_loading: bool,
    pub auto_edit_metadata: bool,
    pub error: Option<String>,
}

enum AddItemMode {
    Browse,
    AddExisting,
    AddNew,
}

struct State {
    title: String,
    description: String,
    items: Vec<Item>,
    registry: SharedRegistry,
    view: AddItemMode,
    metadata_edit: bool,
    is_changed: bool,
}

brass::enable_props!(wrapped CollectionForm => State);

enum Msg {
    Title(String),
    Description(String),
    AddItem(Item),
    RemoveItem(usize),
    ToggleMode(AddItemMode),
    ToggleEditMetadata,
    Submit,
}

fn render_items(items: &[Item], registry: &Registry, on_remove: Callback<usize>) -> Vec<VNode> {
    let opts = semantic_ui_core::EntityRenderOpts {
        editable: false,
        preview: true,
    };

    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let view = EntityView::build(item, &opts, registry);
            let view_wrap = vdom::div().class(s("is-flex-grow-1")).and(view);

            let btn_remove = brass_bulma::button()
                .and(brass_bulma::icon_fa(s("fas fa-minus-circle")))
                .attr(Attr::Title, s("Remove"))
                .on_callback(move |_: ClickEvent| index, &on_remove);
            let actions = vdom::div().class(s("ml-4")).and(btn_remove);

            vdom::div()
                .class(s("is-flex"))
                .and((view_wrap, actions))
                .build()
        })
        .collect()
}

impl PropComponent for State {
    type Properties = CollectionForm;
    type Msg = Msg;

    fn init(props: &Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        let items = props.item.items.clone();

        Self {
            title: props.item.collection.title.clone(),
            description: props
                .item
                .collection
                .description
                .clone()
                .unwrap_or_default(),
            items,
            registry: ctx.registry().clone(),
            metadata_edit: props.auto_edit_metadata,
            view: AddItemMode::Browse,
            is_changed: false,
        }
    }

    fn update(
        &mut self,
        msg: Self::Msg,
        props: &Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) {
        match msg {
            Msg::Title(value) => {
                let changed = value != self.title && !value.trim().is_empty();
                self.title = value;
                if changed {
                    self.is_changed = true;
                }
            }
            Msg::Description(value) => {
                let changed = value != self.description && !value.trim().is_empty();
                self.description = value;
                if changed {
                    self.is_changed = true;
                }
            }
            Msg::Submit => {
                let col = &props.item.collection;

                let title = self.title.trim().to_string();

                if title.is_empty() {
                    return;
                }

                let col = Collection {
                    id: col.id,
                    ident: col.ident.clone(),
                    url: col.url.clone(),
                    title: self.title.clone(),
                    description: if self.description.is_empty() {
                        None
                    } else {
                        Some(self.description.clone())
                    },
                    item_ids: self
                        .items
                        .iter()
                        .filter_map(|item| item.data.get_id())
                        .collect(),
                    extra: Default::default(),
                };
                props.on_submit.send(col);
                self.is_changed = false;
            }
            Msg::AddItem(item) => {
                self.items.push(item);
                self.view = AddItemMode::Browse;
                self.is_changed = true;
            }
            Msg::ToggleMode(mode) => {
                self.view = mode;
            }
            Msg::RemoveItem(index) => {
                self.items.remove(index);
                self.is_changed = true;
            }
            Msg::ToggleEditMetadata => {
                self.metadata_edit = !self.metadata_edit;
            }
        }
    }

    fn render(
        &self,
        props: &Self::Properties,
        ctx: &mut brass::RenderContext<brass::PropWrapper<Self>>,
    ) -> brass::VNode {
        let header_content = if self.metadata_edit {
            let title = brass_bulma::FieldHorizontal {
                label: s("Title"),
                help: None,
                control: brass_bulma::Input {
                    _type: "text".into(),
                    color: brass_bulma::Color::Default,
                    placeholder: None,
                    value: self.title.clone().into(),
                    on_input: ctx.callback_map(Msg::Title),
                },
            };

            let description = brass_bulma::FieldHorizontal {
                label: s("Description"),
                help: None,
                control: brass_bulma::Textarea {
                    color: brass_bulma::Color::Default,
                    placeholder: None,
                    value: self.description.clone().into(),
                    on_input: ctx.callback_map(Msg::Description),
                    on_keydown: None,
                    style_raw: None,
                },
            };

            div_with((title, description))
        } else {
            let description = if self.description.is_empty() {
                vdom::span()
            } else {
                vdom::div()
                    .class(s("message is-flex"))
                    .attr(Attr::Title, s("Description"))
                    .and(vdom::div().class(s("message-body")).and(&self.description))
            };

            let toggle_edit_meta = brass_bulma::button()
                .and(s("Edit Metadata"))
                .on(ctx, |_: ClickEvent| Msg::ToggleEditMetadata);

            vdom::div()
                .class(s("mb-3"))
                .and((description, toggle_edit_meta))
        };

        let meta_edit = vdom::div_with(header_content);

        let item_list = if self.items.is_empty() {
            brass_bulma::notification_warning(s("Collection is empty")).build()
        } else {
            let items_rendered = render_items(
                &self.items,
                &self.registry,
                ctx.callback_map(Msg::RemoveItem),
            );
            vdom::div()
                .class(s("mb-4"))
                .and_iter(items_rendered)
                .build()
        };

        let item_action = match self.view {
            AddItemMode::Browse => {
                let btn_add_existing = brass_bulma::button()
                    .and(brass_bulma::icon_fa(s("fas fa-search-plus")))
                    .and(vdom::span_with(s("Add")))
                    .attr(Attr::Title, s("Add existing entity"))
                    .on(ctx, |_: ClickEvent| {
                        Msg::ToggleMode(AddItemMode::AddExisting)
                    });
                brass_bulma::buttons().and(btn_add_existing).build()
            }
            AddItemMode::AddExisting => {
                // TODO: should probably be cached...
                let mut ignored = HashSet::new();
                ignored.insert(props.item.collection.id);

                for item in &self.items {
                    if let Some(id) = item.data.get_id() {
                        ignored.insert(id);
                    }
                }

                let autocomplete = EntitySearchAutocomplete {
                    placeholder: None,
                    renderer: None,
                    attribute: None,
                    filter: None,
                    on_select: ctx.callback_map(Msg::AddItem),
                    ignored_ids: Some(ignored),
                };
                brass_bulma::box_()
                    .and((vdom::div().and(s("Find")), autocomplete))
                    .build()
            }
            AddItemMode::AddNew => {
                todo!()
            }
        };

        let item_action_wrap = vdom::div().class(s("mb-4")).and(item_action);

        let items = vdom::div().and((
            vdom::hr(),
            vdom::div().and(vdom::b().and(s("Items"))).class(s("mb-4")),
            item_list,
            item_action_wrap,
            vdom::hr(),
        ));

        let error_notification = props
            .error
            .as_ref()
            .map(|x| semantic_ui_core::loader::error_msg(&x));

        let submit = brass_bulma::button()
            .and_class("is-primary")
            .and(if props.is_loading {
                s("...")
            } else {
                s("Save")
            })
            .attr_toggle_if(props.is_loading || !self.is_changed, Attr::Disabled)
            .on(ctx, |_: ClickEvent| Msg::Submit);
        let actions = brass_bulma::buttons().and(submit);

        vdom::div()
            .and((meta_edit, items, error_notification, actions))
            .build()
    }
}
