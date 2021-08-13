use std::{collections::HashSet, rc::Rc};

use brass::{
    vdom::{self, EventCallback, Render},
    Callback, VNode,
};
use semantic_ui_core::components::form::{
    self, AndValidator, FormRef, InputField, StringRequired, Validator,
};
use semantics_core::base::Tag;

#[derive(Clone, Debug)]
pub struct ExistingTagValidator {
    pub tags: HashSet<String>,
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

pub fn tag_form(
    tag: Tag,
    on_submit: Callback<Tag>,
    existing_validtor: Option<Rc<ExistingTagValidator>>,
) -> VNode {
    form::Form {
        on_submit,
        initial_values: tag,
        render: Rc::new(move |mut state: FormRef<Tag>| {
            let name_validator = StringRequired;
            let name_validator = if let Some(existing) = &existing_validtor {
                AndValidator::new(name_validator)
                    .and(existing.clone())
                    .boxed()
            } else {
                name_validator.boxed()
            };

            vdom::div()
                .and(state.field(InputField::<Tag> {
                    name: "name".to_string(),
                    get: |h| &h.name,
                    set: |v, h| {
                        h.name = v;
                    },
                    validate: Some(name_validator),
                    label: "Title".to_string(),
                    help: None,
                    placeholder: None,
                }))
                .and(
                    vdom::div().and(
                        brass_bulma::button()
                            .and_class("is-primary")
                            .and("Create")
                            .on_click(EventCallback::callback(|_| (), state.submit())),
                    ),
                )
                .build()
        }),
    }
    .render()
}
