use std::rc::Rc;

use brass::dom::TagBuilder;
use semantic_core::api::{BackendConfig, BackendCryptoConfig, DbConfig, SemanticSchema};
use semantic_ui_core::{
    components::{
        form,
        util::{form_field_input, FormBuilder},
    },
    context,
    validate::StringRequired,
};

struct Values {
    data_path: String,
    key: String,
}

pub fn login(on_success: impl Fn(SemanticSchema) + 'static) -> TagBuilder {
    let on_success: Rc<dyn Fn(SemanticSchema)> = Rc::new(on_success);

    form::Form::new(Values {
        data_path: String::new(),
        key: String::new(),
    })
    .on_submit_async(move |values| {
        let key = values.key.clone();
        let data_path = values.data_path.clone();
        let on_success = on_success.clone();

        Box::pin(async move {
            let schema = context::api()
                .initialize(BackendConfig {
                    db: DbConfig::Crypto(BackendCryptoConfig {
                        data_path: Some(data_path),
                        key,
                    }),
                    idle_timeout: None,
                })
                .await?;

            on_success(schema);

            Ok(())
        })
    })
    .render(|handle| {
        FormBuilder::new(handle.clone())
            .and(form_field_input(
                "Data Path",
                handle.field_validated(
                    |v| &mut v.data_path,
                    StringRequired,
                ),
            ))
            .and(form_field_input(
                "Key",
                handle.field_validated(|v| &mut v.key, StringRequired),
            ))
            .buttons_submit("Log in")
    })
}
