use brass::dom::TagBuilder;

use semantic_core::base::Collection;

use crate::{
    components::{
        form::{self, FormLoadFuture},
        util::{form_field_input, FormRenderer},
    },
    validate::{StringRequired, ValidateOptionalStr, ValidateStringUrl},
};

#[derive(Clone)]
struct Values {
    title: String,
    description: String,
    url: String,
}

pub fn collection_metadata_form(
    col: Collection,
    on_submit_async: impl Fn(Collection) -> FormLoadFuture + 'static,
) -> TagBuilder {
    form::Form::new(Values {
        title: col.title.clone(),
        description: col.description.clone().unwrap_or_default(),
        url: col.url.as_ref().map(|x| x.to_string()).unwrap_or_default(),
    })
    .on_submit_async(move |values| {
        tracing::trace!("collection for submit");
        let mut col = col.clone();
        col.title = values.title.clone();
        col.description = if values.description.trim().is_empty() {
            None
        } else {
            Some(values.description.trim().into())
        };
        col.url = if values.url.is_empty() {
            None
        } else {
            // NOTE: unwrap because form validator ensures valid value.
            url::Url::parse(&values.url).ok()
        };
        on_submit_async(col)
    })
    .render(move |handle| {
        let title = form_field_input(
            "Title",
            handle.field_validated(|v| &mut v.title, StringRequired),
        );

        let description = form_field_input("Description", handle.field(|v| &mut v.description));

        let url = form_field_input(
            "Url",
            handle.field_validated(|v| &mut v.url, ValidateOptionalStr(ValidateStringUrl)),
        );

        FormRenderer::new(handle.clone())
            .and(title)
            .and(description)
            .and(url)
            .buttons_submit("Save")
    })
}
