use std::rc::Rc;

use brass::{
    dom::{
        builder::{div, span},
        Tag, TagBuilder,
    },
    signal::{
        signal::{Mutable, SignalExt},
        signal_vec::MutableVec,
    },
};
use factordb::{
    data::value::patch::Patch,
    query::select::Page,
    schema::{builtin::AttrIdent, AttributeDescriptor},
    AnyError, Id,
};
use semantic_core::{
    api::PluginTestFetch,
    core::{AttrPluginCode, PluginSource},
    plugin::ImportOutput,
};
use semantic_ui_core::{
    components::{
        form::{self, FormLoadFuture},
        loader::{load, spinner, LoadState, Loader},
        util::{
            buttons, form_field_input, form_field_textarea, notification_default,
            notification_error, notification_warning, subtitle_4, title_2, ButtonBuilder,
            FormBuilder,
        },
    },
    context,
    routing::Route,
    validate::{StringRequired, StringUrl},
};

async fn load_all_sources() -> Result<Page<PluginSource>, AnyError> {
    context::api()
        .select_entities(PluginSource::query_all())
        .await
}

fn plugin_source_deleter(
    source: PluginSource,
    on_deleted: impl Fn() + 'static,
    on_cancel: impl Fn() + 'static,
) -> TagBuilder {
    let loader = Loader::new_idle();

    let loader2 = loader.clone();
    let on_cancel = Rc::new(on_cancel);
    let on_deleted = Rc::new(on_deleted);

    let signal = loader2.signal_render_state(move |state| match state {
        LoadState::Idle => {
            let loader = loader.clone();
            let plugin_name = source.ident.clone();
            let on_cancel = on_cancel.clone();
            let on_deleted = on_deleted.clone();

            notification_warning()
                .and(format!("Really delete plugin {}", source.ident))
                .and(
                    ButtonBuilder::new()
                        .color(semantic_ui_core::components::util::Color::Danger)
                        .label("Delete")
                        .on(move || {
                            let plugin_name = plugin_name.clone();
                            let on_deleted = on_deleted.clone();
                            loader.spawn(async move {
                                context::api().plugin_delete(plugin_name).await?;
                                on_deleted();
                                Ok(())
                            });
                        })
                        .build(),
                )
                .and(
                    ButtonBuilder::new()
                        .label("Cancel")
                        .on(move || {
                            on_cancel();
                        })
                        .build(),
                )
        }
        LoadState::Loading(_) => spinner(),
        LoadState::Success(_) => div(),
        LoadState::Failed(err) => {
            let on_cancel = on_cancel.clone();

            notification_error().and(err.to_string()).and(
                ButtonBuilder::new()
                    .label("Cancel")
                    .on(move || {
                        on_cancel();
                    })
                    .build(),
            )
        }
    });

    div().child_signal(signal)
}

pub fn plugin_manager() -> TagBuilder {
    let actions = buttons()
        .and(
            ButtonBuilder::new()
                .label("Create Plugin")
                .on(|| context::router().goto(Route::PluginCreate))
                .build(),
        )
        .and(
            ButtonBuilder::new()
                .label("Test Plugin Code")
                .on(|| context::router().goto(Route::PluginTest))
                .build(),
        );

    let list = load(load_all_sources(), |page| {
        if page.items.is_empty() {
            notification_warning().and("No plugins found.")
        } else {
            let items = MutableVec::new_with_values(page.items.clone());

            div()
                .children_signal(items.signal_vec_cloned(), |source| {
                    let deleting = Mutable::new(false);
                    let deleting2 = deleting.clone();

                    let source = source.clone();

                    div()
                        .and(subtitle_4().and(&source.ident))
                        .and(
                            ButtonBuilder::new()
                                .label("Delete")
                                .on(move || {
                                    deleting2.replace_with(|x| !*x);
                                })
                                .build(),
                        )
                        .child_signal(deleting.signal().map(move |is_deleting| {
                            let deleting = deleting.clone();
                            if is_deleting {
                                plugin_source_deleter(
                                    source.clone(),
                                    || {
                                        context::router().goto(Route::PluginManager);
                                    },
                                    move || {
                                        deleting.set(false);
                                    },
                                )
                            } else {
                                span()
                            }
                        }))
                        .build()
                })
                .bind(items)
        }
    });

    div().and(title_2().and("Plugins")).and(actions).and(list)
}

#[derive(Clone)]
struct FormValues {
    pub name: String,
    pub code: String,
}

impl FormValues {
    fn apply(self, source: &mut PluginSource) {
        source.ident = self.name;
        source.code = Some(self.code);
    }

    fn build_patch(self, old: &PluginSource) -> Patch {
        let mut patch = Patch::new();
        if self.name != old.ident {
            patch = patch.replace(AttrIdent::QUALIFIED_NAME, self.name);
        }
        if Some(&self.code) != old.code.as_ref() {
            patch = patch.replace(AttrPluginCode::QUALIFIED_NAME, self.code);
        }

        patch
    }
}

fn plugin_source_form(
    source: PluginSource,
    is_new: bool,
    on_submit_async: impl Fn(FormValues) -> FormLoadFuture + 'static,
) -> TagBuilder {
    form::Form::new(FormValues {
        name: source.ident,
        code: source.code.unwrap_or_default(),
    })
    .on_submit_async(move |values| on_submit_async(values.clone()))
    .render(move |handle| {
        let mut builder = FormBuilder::new(handle.clone());

        if is_new {
            let name = form_field_input(
                "Name",
                handle.field_validated(|v| &mut v.name, StringRequired),
            );
            builder = builder.and(name);
        }

        let code = form_field_textarea(
            "Code",
            handle.field_validated(|v| &mut v.code, StringRequired),
            5,
            true,
        );

        builder.and(code).buttons_submit("Save")
    })
}

fn plugin_source_create() -> TagBuilder {
    let source = PluginSource {
        id: Id::random(),
        ident: String::new(),
        runtime: Some("deno".to_string()),
        code: None,
    };

    plugin_source_form(source.clone(), true, move |values| {
        let mut source = source.clone();
        values.apply(&mut source);

        Box::pin(async move {
            context::api().plugin_source_create(source.clone()).await?;
            context::router().goto(Route::PluginManager);
            Ok(())
        })
    })
}

pub fn plugin_source_create_page() -> TagBuilder {
    div()
        .and(title_2().and("Create Plugin"))
        .and(plugin_source_create())
}

pub fn plugin_test_page() -> TagBuilder {
    #[derive(Clone)]
    struct Values {
        code: String,
        url: String,
    }

    let result: Loader<Option<ImportOutput>> = Loader::new_idle();
    let result2 = result.clone();

    form::Form::new(Values {
        code: String::new(),
        url: String::new(),
    })
    .on_submit_async(move |values| {
        let mut result = result.clone();
        let values = values.clone();

        Box::pin(async move {
            let out = context::api()
                .plugin_test_fetch(PluginTestFetch {
                    runtime: "deno".into(),
                    // Unwrap is fine because form logic handles validation.
                    url: values.url.parse().unwrap(),
                    code: values.code,
                })
                .await;
            result.set_result(out);

            Ok(())
        })
    })
    .render(move |handle| {
        let code = form_field_textarea(
            "Code",
            handle.field_validated(|v| &mut v.code, StringRequired),
            5,
            true,
        );

        let url = form_field_input("Url", handle.field_validated(|v| &mut v.url, StringUrl));

        let form = FormBuilder::new(handle.clone())
            .and(code)
            .and(url)
            .buttons_submit("Test");

        let output_signal = result2.signal_render(|opt| {
            if let Some(out) = opt {
                Tag::Pre.new().and(
                    serde_json::to_string_pretty(&out)
                        .unwrap_or_else(|_| "Invalid JSON".to_string()),
                )
            } else {
                notification_warning().and("Plugin did not return any import results.")
            }
        });
        let output = div()
            .style_raw("margin-top: 2rem;")
            .child_signal(output_signal);

        div()
            .and(title_2().and("Test Plugin"))
            .and(notification_default().and(
                "Enter the plugin source code and a URL. The import result will be displayed.",
            ))
            .and(form)
            .and(output)
    })
}
