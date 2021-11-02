use std::rc::Rc;

use brass::dom::{Render, TagBuilder};
use factordb::AnyError;
use futures::future::LocalBoxFuture;
use semantic_core::base::Collection;

use crate::components::autocomplete::multiselect::{multiselect_render_tags, MultiSelect};

pub struct CollectionSelect {
    pub initial_selection: Vec<Collection>,
    pub on_add_async:
        Option<Rc<dyn Fn(Collection) -> LocalBoxFuture<'static, Result<Collection, AnyError>>>>,
    pub on_remove_async:
        Option<Rc<dyn Fn(Collection) -> LocalBoxFuture<'static, Result<Collection, AnyError>>>>,
}

impl Render for CollectionSelect {
    fn render(self) -> TagBuilder {
        MultiSelect::<Collection> {
            heading: "Select Collections".to_string(),
            get_id: |t| t.id.to_string(),
            options: Vec::new(),
            initial_selection: self.initial_selection.clone(),
            multi: true,
            search: Some(Box::new(move |term| {
                Box::pin(async move {
                    let page = super::search_collections(term).await?;
                    Ok(page.items)
                })
            })),
            load_more: None,
            on_change: None,
            on_submit: None,
            on_add_async: self.on_add_async.clone(),
            on_change_async: None,
            on_remove_async: self.on_remove_async.clone(),
            on_cancel: None,
            render: Box::new(|args| multiselect_render_tags(args, |t| &t.title)),
            selected_fallback: None,
            available_fallback: None,
        }
        .render()
    }
}
