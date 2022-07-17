use brass::{
    component::{msg::MsgComponent, Component},
    dom::{builder::div, Tag, TagBuilder, View},
    signal::{signal::Mutable, signal_vec::MutableVec},
};
use factordb::prelude::{Id, Mutate};
use semantic_ui_core::{
    components::{
        loader::{LoadState, Loader},
        util::{
            buttons, focus, notification_error, notification_warning, table, title_2, ButtonBuilder,
        },
    },
    context::api,
};

use super::WeightLogEntry;

pub struct WeightlogManager {}

impl brass::dom::Render for WeightlogManager {
    fn render(self) -> View {
        State::build(self)
    }
}

enum Msg {
    DeleteEntry(Id),
    DeleteErrorDismiss,
    EntryDeleted(Id),

    Create,
    Created(WeightLogEntry),
}

struct State {
    // TODO: handle pagination
    entries: Loader<MutableVec<WeightLogEntry>>,

    delete_loader: Loader<()>,

    creating: Mutable<bool>,
}

impl MsgComponent for State {
    type Properties = WeightlogManager;
    type Msg = Msg;

    fn init(_props: Self::Properties, _ctx: brass::component::Context<Self>) -> Self {
        Self {
            entries: Loader::new_spawn(async {
                let items = api().select_entities(WeightLogEntry::query_all()).await?;
                Ok(MutableVec::new_with_values(items))
            }),
            delete_loader: Loader::new_idle(),
            creating: Mutable::new(false),
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: brass::component::Context<Self>) {
        match msg {
            Msg::DeleteEntry(id) => {
                if self.delete_loader.is_loading() {
                    return;
                }
                let on_deleted = ctx.on(|id| Msg::EntryDeleted(id));
                self.delete_loader.spawn(async move {
                    api().mutate(Mutate::delete(id)).await?;

                    on_deleted(id);

                    Ok(())
                });
            }
            Msg::DeleteErrorDismiss => {
                self.delete_loader.set_idle();
            }
            Msg::EntryDeleted(id) => match &*self.entries.get().lock_mut() {
                LoadState::Success(items) => {
                    let mut items = items.lock_mut();
                    items.retain(|item| item.id != id);
                }
                _ => {}
            },
            Msg::Create => {
                self.creating.set(true);
            }
            Msg::Created(entry) => match &mut *self.entries.get().lock_mut() {
                LoadState::Success(items) => {
                    items.lock_mut().insert_cloned(0, entry);
                }
                _ => {}
            },
        }
    }

    fn render(&mut self, ctx: brass::component::Context<Self>) -> TagBuilder {
        let handle = ctx.handle();
        let delete_error = self
            .delete_loader
            .signal_render_state(move |state| -> View {
                if let LoadState::Failed(err) = state {
                    let err = notification_error()
                        .and(Tag::P.new().and("Could not delete entry."))
                        .and(Tag::P.new().and(err.as_str()))
                        .and(
                            div().and(
                                ButtonBuilder::new()
                                    .label("Ok")
                                    .on(handle.callback(|| Msg::DeleteErrorDismiss))
                                    .build(),
                            ),
                        );

                    focus(err).into()
                } else {
                    View::Empty
                }
            });

        let handle = ctx.handle();
        let items = self.entries.signal_render(move |items| -> View {
            let handle = handle.clone();
            table()
                .and(
                    Tag::Thead.new().and(
                        Tag::Tr
                            .new()
                            .and(Tag::Th.new().and("Date"))
                            .and(Tag::Th.new().and("Weight"))
                            .and(Tag::Th.new().and("Actions")),
                    ),
                )
                .and(
                    Tag::Tbody.new().signal_vec_with_fallback(
                        items.signal_vec_cloned(),
                        move |item| {
                            let id = item.id;
                            Tag::Tr
                                .new()
                                .and(Tag::Td.new().and(
                                    item.datetime.to_datetime().format("%Y-%m-%d").to_string(),
                                ))
                                .and(Tag::Td.new().and(item.weight.to_string()))
                                .and(
                                    Tag::Td.new().and(
                                        buttons().and(
                                            ButtonBuilder::new()
                                                .size_small()
                                                .label("Delete")
                                                .on(handle.callback(move || Msg::DeleteEntry(id)))
                                                .build(),
                                        ),
                                    ),
                                )
                                .build()
                        },
                        notification_warning()
                            .class("mt-4")
                            .and("No weight entries yet."),
                    ),
                )
                .into()
        });

        let actions = buttons().and(
            ButtonBuilder::new()
                .label("Log Weight")
                .signal_disabled(self.creating.signal())
                .on(ctx.callback_msg(|| Msg::Create))
                .build(),
        );

        let handle = ctx.handle();
        let form = self.creating.signal_ref(move |is_creating| {
            if !*is_creating {
                View::Empty
            } else {
                super::weightlog_create(handle.on(Msg::Created)).into()
            }
        });

        div()
            .and(title_2().and("Weight"))
            .and(actions)
            .signal(form)
            .signal(delete_error)
            .signal(items)
    }
}
