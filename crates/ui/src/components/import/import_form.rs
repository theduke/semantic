use brass::dom::TagBuilder;
use semantic_ui_core::{
    components::{
        form::{Form, FormHandle, FormLoadFuture},
        util::{form_field_checkbox, form_field_input, FormRenderer},
    },
    validate::StringUrl,
};

#[derive(Clone)]
pub struct Values {
    pub url: String,
    pub import_media: bool,
    pub import: bool,
}

impl Default for Values {
    fn default() -> Self {
        Self {
            url: Default::default(),
            import_media: true,
            import: false,
        }
    }
}

pub fn import_form_new() -> Form<Values> {
    Form::new(Values::default())
}

pub fn import_form_render(handle: FormHandle<Values>) -> TagBuilder {
    FormRenderer::new(handle.clone())
        .and(form_field_input(
            "Url",
            handle.field_validated(|v| &mut v.url, StringUrl),
        ))
        .and(form_field_checkbox(
            "Import Media",
            handle.field(|v| &mut v.import_media),
        ))
        .and(form_field_checkbox(
            "Import (persist)",
            handle.field(|v| &mut v.import),
        ))
        .with_buttons(|b| b.submit("Load").reset_default("Reset"))
        .build()
}
