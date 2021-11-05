use brass::dom::{builder::div, Render, TagBuilder};
use semantic_core::plugin::ImportOutput;
use semantic_ui_core::{
    components::{
        entity::{entity_box::EntityBox, entity_list},
        loader::Loader,
        util::{buttons, notification_success, notification_warning, title_2, ButtonBuilder, Cls},
    },
    context::{self, api},
    EntityRenderOpts,
};

use url::Url;

#[derive(Clone)]
struct Inner {
    url: Url,
    is_imported: bool,
    import_media: bool,
    output: Option<ImportOutput>,
}

pub fn import_page() -> TagBuilder {
    let loader = Loader::<Inner>::new_idle();

    let loader2 = loader.clone();
    let form = super::import_form::import_form(move |values| {
        let values = values.clone();
        let mut loader = loader2.clone();
        Box::pin(async move {
            let url = url::Url::parse(&values.url)?;
            let out = api()
                .fetch_url(url.clone(), values.import, values.import_media)
                .await?;

            loader.set_result(Ok(Inner {
                output: out,
                url,
                is_imported: values.import,
                import_media: values.import_media,
            }));

            Ok(())
        })
    });

    let content = loader
        .clone()
        .signal_render(move |inner| match &inner.output {
            None => notification_warning().and("Nothing found."),
            Some(output) if output.items.is_empty() => notification_warning().and("Nothing found."),
            Some(output) if !inner.is_imported => {
                let inner = inner.clone();
                let loader = loader.clone();

                let import_btn = ButtonBuilder::new()
                    .size_medium()
                    .label("Import all")
                    .on(move || {
                        let inner = inner.clone();
                        loader.spawn(async move {
                            let output = api()
                                .clone()
                                .fetch_url(inner.url.clone(), true, inner.import_media)
                                .await?;
                            Ok(Inner {
                                output,
                                url: inner.url,
                                is_imported: true,
                                import_media: inner.import_media,
                            })
                        });
                    })
                    .build();

                let actions = buttons().class("mb-4").and(import_btn);

                let list = entity_list(
                    &output.items,
                    &context::registry(),
                    &EntityRenderOpts {
                        editable: false,
                        preview: true,
                    },
                );

                div().and(actions).and(list)
            }
            Some(output) => div()
                .and(notification_success().and(format!("Imported {} items", output.items.len())))
                .and_iter(output.items.iter().map(|item| {
                    EntityBox {
                        item: item.clone(),
                        options: EntityRenderOpts {
                            editable: false,
                            preview: true,
                        },
                        on_delete: None,
                    }
                    .render()
                })),
        });

    div()
        .and(title_2().and("Import"))
        .and(form.class("mb-4").class(Cls::Box))
        .child_signal(content)
}
