use std::rc::Rc;

use brass::{
    vdom::{s, Func, Render},
    Callback, Shared, VNode,
};
use semantic_core::base::Collection;
use semantic_ui_core::{
    components::{autocomplete::multiselect, Loader, LoaderFuture, WithApi},
    loader::LoadState,
};

pub fn collection_picker(on_select: Callback<Option<Collection>>) -> VNode {
    let render = Rc::new(
        |args: multiselect::MultiSelectRender<Collection>| -> VNode {
            multiselect::multiselect_render_tags(args, |col| &col.title)
        },
    );
    let on_select = on_select.map(|mut items: Vec<Collection>| items.pop());

    WithApi {
        render: brass::vdom::RefFunc::dynamic(
            move |api: &semantic_ui_core::api::BrowserApiClient| {
                let on_select = on_select.clone();
                let render = render.clone();

                let api = api.clone();
                Loader::<(), Vec<Collection>> {
                    input: (),
                    load: Func::dynamic(move |_: ()| -> LoaderFuture<Vec<Collection>> {
                        let api = api.clone();
                        Box::pin(async move {
                            let query = Collection::query_all_collections();
                            let page = api.select_entities::<Collection>(query).await?;
                            Ok(page.items)
                        })
                    }),
                    render: Func::dynamic(move |items| {
                        multiselect::MultiSelect::<Collection> {
                            heading: s("Select Collection"),
                            compare_identity: |t1, t2| t1.id == t2.id,
                            options: items,
                            load_status: LoadState::Success(()),
                            initial_selection: Shared::new(Vec::new()),
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
