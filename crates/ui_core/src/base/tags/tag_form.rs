use std::{collections::HashSet, rc::Rc};

use brass::dom::TagBuilder;

use semantic_core::base::Tag;

use crate::{
    components::{
        form::{self, FormLoadFuture},
        util::{form_field_input, FormRenderer},
    },
    validate::{AndValidator, StringRequired, Validator},
};

#[derive(Clone, Debug, Default)]
pub struct ExistingTagValidator {
    pub tags: HashSet<String>,
}

impl ExistingTagValidator {
    pub fn from_tags(tags: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        Self {
            tags: tags.into_iter().map(|x| x.as_ref().to_string()).collect(),
        }
    }
}

impl Validator<String> for ExistingTagValidator {
    fn validate(&self, value: &String) -> Result<(), Vec<String>> {
        if self.tags.contains(value.trim()) {
            Err(vec!["Tag already exists".into()])
        } else {
            Ok(())
        }
    }
}

#[derive(Clone)]
struct Values {
    name: String,
}

pub fn tag_form(
    tag: Tag,
    on_submit_async: impl Fn(Tag) -> FormLoadFuture + 'static,
    // TODO: on_cancel
    existing_validator: Rc<ExistingTagValidator>,
) -> TagBuilder {
    form::Form::new(Values {
        name: String::new(),
    })
    .on_submit_async(move |values| {
        let mut tag = tag.clone();
        tag.name = values.name.clone();
        on_submit_async(tag)
    })
    .render(move |handle| {
        let name = form_field_input(
            "Name",
            handle.field_validated(
                |v| &mut v.name,
                AndValidator::new(StringRequired).and(existing_validator.boxed()),
            ),
        );

        FormRenderer::new(handle.clone())
            .and(name)
            .buttons_submit("Save")
    })
}
