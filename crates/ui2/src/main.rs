#![allow(non_snake_case)]

mod api;
mod cmp;

use cmp::app::App;

fn main() {
    dioxus_web::launch(App);
}

// create a component that renders a div with the text "Hello, world!"
// fn App(cx: Scope) -> Element {
//     // let items_res = use_future(cx, (), |_inp| {
//     //     async move {
//     //     let api = crate::api::new_api(Some("http://localhost:3000/api/query".to_string()));
//     //         api.select(Select {
//     //             ..Default::default()
//     //         })
//     //         .await
//     //     }
//     // });

//     // let out = match items_res.value() {
//     //     Some(Ok(items)) => {
//     //         let count = items.len();

//     //         cx.render(rsx! {
//     //             "ITEMS: {count}"
//     //         })
//     //     }
//     //     Some(Err(err)) => cx.render(rsx! {
//     //         "Error: {err}"
//     //     }),
//     //     None => cx.render(rsx! {
//     //         "loading..."
//     //     }),
//     // };

//     // cx.render(rsx! {
//     //     div {
//     //         class: "container",
//     //         out

//     //         Browser {}

//     //         Alert {
//     //             color: cmp::bootstrap::Color::Danger,
//     //             "OH NOES"
//     //         }
//     //     }
//     // })

//     cx.render(rsx! {
//         App {}
//     })
// }
