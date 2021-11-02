use std::rc::Rc;

use brass::dom::{Render, TagBuilder};
use factordb::AnyError;
use futures::future::LocalBoxFuture;
use semantic_core::base::Tag;

use crate::components::{
    autocomplete::multiselect::{multiselect_render_tags, Action, MultiSelect},
    loader::load,
};

async fn load_all_tags() -> Result<Vec<Tag>, AnyError> {
    let page = crate::context::api()
        .select_entities::<Tag>(Tag::query_all())
        .await?;
    Ok(page.items)
}

pub struct TagSelect {
    pub initial_selection: Vec<Tag>,
    pub on_submit: Option<Rc<dyn Fn(&Vec<Tag>)>>,
    pub on_change: Option<Rc<dyn Fn(Vec<Tag>)>>,
    pub on_add_async: Option<Rc<dyn Fn(Tag) -> LocalBoxFuture<'static, Result<Tag, AnyError>>>>,
    pub on_remove_async: Option<Rc<dyn Fn(Tag) -> LocalBoxFuture<'static, Result<Tag, AnyError>>>>,
    pub on_change_async:
        Option<Rc<dyn Fn(Vec<Tag>) -> LocalBoxFuture<'static, Result<Vec<Tag>, AnyError>>>>,
}

impl Render for TagSelect {
    fn render(self) -> TagBuilder {
        load(load_all_tags(), move |tags| {
            let all_tags = Rc::new(tags.clone());
            MultiSelect::<Tag> {
                heading: "Select Tags".to_string(),
                get_id: |t| t.id.to_string(),
                options: tags.clone(),
                initial_selection: self.initial_selection.clone(),
                multi: true,
                search: Some(Box::new(move |term| {
                    let all_tags = all_tags.clone();
                    Box::pin(async move {
                        let items = all_tags
                            .iter()
                            .filter(|tag| {
                                tag.name
                                    .to_lowercase()
                                    .contains(&term.trim().to_lowercase())
                            })
                            .cloned()
                            .collect();
                        Ok(items)
                    })
                })),
                load_more: None,
                on_change: self.on_change.clone(),
                on_submit: self.on_submit.clone().map(|f| Action {
                    label: "Save".to_string(),
                    on: f.clone(),
                }),
                on_add_async: self.on_add_async.clone(),
                on_change_async: self.on_change_async.clone(),
                on_remove_async: self.on_remove_async.clone(),
                on_cancel: None,
                render: Box::new(|args| multiselect_render_tags(args, |t| &t.name)),
                selected_fallback: None,
                available_fallback: None,
            }
            .render()
        })
    }
}
