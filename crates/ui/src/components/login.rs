use std::rc::Rc;

use brass::dom::{builder::div, TagBuilder};
use semantic_core::api::{BackendConfig, BackendCryptoConfig, DbConfig, SemanticSchema};
use semantic_ui_core::{
    components::{
        form,
        loader::Loader,
        util::{
            form_field_checkbox, form_field_input, form_field_password, subtitle_4, title_2,
            FormRenderer,
        },
    },
    context,
    validate::StringRequired,
};

#[derive(Clone)]
struct Values {
    data_path: String,
    key: String,
    raw: bool,
}

pub fn login(on_success: impl Fn(SemanticSchema) + 'static) -> TagBuilder {
    let on_success: Rc<dyn Fn(SemanticSchema)> = Rc::new(on_success);

    let form = form::Form::new(Values {
        data_path: String::new(),
        key: String::new(),
        raw: false,
    })
    .on_submit_async(move |values| {
        let key = values.key.clone();
        let data_path = values.data_path.clone();
        let raw = values.raw;
        let on_success = on_success.clone();

        Box::pin(async move {
            let schema = context::api()
                .initialize(BackendConfig {
                    db: DbConfig::Crypto(BackendCryptoConfig {
                        data_path: Some(data_path),
                        key,
                        raw,
                        key_iterations: None,
                        salt: None,
                    }),
                    idle_timeout: None,
                })
                .await?;

            on_success(schema);

            Ok(())
        })
    })
    .render(|handle| {
        FormRenderer::new(handle.clone())
            .and(form_field_input(
                "Data Path",
                handle.field_validated(|v| &mut v.data_path, StringRequired),
            ))
            .and(form_field_password(
                "Key",
                handle.field_validated(|v| &mut v.key, StringRequired),
            ))
            .and(form_field_checkbox(
                "Raw mode",
                handle.field(|v| &mut v.raw),
            ))
            .buttons_submit("Log in")
    });

    div()
        .class("container")
        .and(title_2().and("Semantic"))
        .and(form)
}

pub fn logout() -> TagBuilder {
    let load = Loader::new_spawn(async {
        context::api().close_backend().await?;
        web_sys::window().unwrap().location().replace("/").ok();
        Ok(())
    })
    .signal_render(|_| div());
    div()
        .and(subtitle_4().and("Logging out..."))
        .child_signal(load)
}
