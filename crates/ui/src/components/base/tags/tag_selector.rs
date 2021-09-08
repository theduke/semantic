use std::rc::Rc;

use brass::{
    vdom::{s, Func, Render},
    Callback, Shared, VNode,
};
use factordb::query::select::Page;
use semantic_core::base::Tag;
use semantic_ui_core::{
    components::{
        autocomplete::multiselect::{self, MultiSelect},
        Loader, LoaderFuture, WithApi,
    },
    loader::LoadState,
};

async fn load_all_tags(
    api: semantic_ui_core::api::BrowserApiClient,
) -> Result<Page<Tag>, anyhow::Error> {
    api.select_entities::<Tag>(Tag::query_all()).await
}

pub fn tag_select(selected: Shared<Vec<Tag>>, on_select: Callback<Vec<Tag>>) -> VNode {
    let render = Rc::new(|args: multiselect::MultiSelectRender<Tag>| -> VNode {
        multiselect::multiselect_render_tags(args, |tag| &tag.name)
    });

    WithApi {
        render: brass::vdom::RefFunc::dynamic(
            move |api: &semantic_ui_core::api::BrowserApiClient| {
                let selected = selected.clone();
                let on_select = on_select.clone();
                let render = render.clone();

                let api = api.clone();
                Loader::<(), Vec<Tag>> {
                    input: (),
                    load: Func::dynamic(move |_: ()| -> LoaderFuture<Vec<Tag>> {
                        let api = api.clone();
                        Box::pin(async move {
                            let page = load_all_tags(api.clone()).await?;
                            Ok(page.items)
                        })
                    }),
                    render: Func::dynamic(move |items| {
                        MultiSelect::<Tag> {
                            heading: s("Select Tags"),
                            compare_identity: |t1, t2| t1.id == t2.id,
                            options: items,
                            load_status: LoadState::Success(()),
                            initial_selection: selected.clone(),
                            multi: true,
                            on_search: None,
                            load_more: None,
                            on_select: Some(on_select.clone()),
                            on_submit: None,
                            on_cancel: None,
                            render: render.clone(),
                        }
                        .render()
                    }),
                }
                .render()
            },
        ),
    }
    .render()
}
