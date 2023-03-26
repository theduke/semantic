use dioxus::prelude::*;
use factdb::DataMap;

use crate::cmp::load::Loader;

pub struct SqlQuery {
    query: String,
}

pub fn Browser<'a>(cx: Scope<'a>) -> Element<'_> {
    let name = use_state(cx, || "bob".to_string());

    let items = use_state::<Loader<Vec<DataMap>>>(cx, || Loader::Idle);

    let submit = move |_| {
        let q = name.get().trim().to_string();
        if q.is_empty() {
            return;
        }

        let items = items.to_owned();

        items.set(Loader::Loading);

        cx.spawn(async move {
            let api = crate::api::new_api(Some("http://localhost:3000/api/query".to_string()));
            let res = api
                .select_sql(q.to_string())
                .await
                .map_err(|e| e.to_string());
            items.set(res.into());
        });
    };

    cx.render(rsx!(div{
        div {
            div {
                class: "mb-3",
                label {
                    class: "form-label",
                    "SQL"
                }
                textarea {
                    class: "form-control",
                    value: "{name}",
                    oninput: move |evt| name.set(evt.value.clone()),
                }
            }

            div {
                button {
                    r#type: "button",
                    class: "btn btn-primary",
                    onclick: submit,
                    if items.is_loading() { "Loading..." } else { "Submit" }
                }
            }


            {items.render(cx, |cx, items| {
                let count = items.len();
                cx.render(rsx!(
                        "items: {count}"

                ))
            })
            }
        }
    }))
}
