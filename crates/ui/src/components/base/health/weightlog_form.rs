// use std::rc::Rc;

// use brass::{Callback, Shared, VNode, vdom::{self, event::ClickEvent, s, Render}};
// use semantic_core::base::WeightLogEntry;
// use semantic_ui_core::components::form::{self, InputField, SelectField};

// struct Values {
//     weight: String,
//     datetime: String,
// }

// pub fn weightlog_form(entry: Option<Shared<WeightLogEntry>>, on_submit: Callback<WeightLogEntry>) -> VNode {
//     form::Form {
//         on_submit,
//         initial_values: Values{
//             weight: entry.weight
//         },
//         render: Rc::new(|mut state| {
//             vdom::div()
//                 // Title.
//                 .and(state.field(InputField::<WeightLogEntry> {
//                     name: s("weight"),
//                     get: |h| &h.weight.to_string(),
//                     set: |v, h| {
//                         h.weight = v;
//                     },
//                     validate: None,
//                     label: s("Title"),
//                     help: None,
//                     placeholder: None,
//                 }))
//                 // Description
//                 .and(state.field(InputField::<WeightLogEntry> {
//                     name: s("time"),
//                     get: |h| &h.datetime.to_datetime().to_string(),
//                     set: |v, h| {
//                         h.datetime = v;
//                     },
//                     validate: None,
//                     label: s("Description"),
//                     help: None,
//                     placeholder: None,
//                 }))
//                 // Mode
//                 .and(
//                     state.field(SelectField::<WeightLogEntry, WeightLogEntryMode> {
//                         name: s("mode"),
//                         get: |h| &h.mode,
//                         set: |v, h| {
//                             h.mode = v;
//                         },
//                         validate: None,
//                         label: s("Description"),
//                         help: None,
//                         // FIXME: don't clone...
//                         options: Rc::new(vec![
//                             brass_bulma::SelectOption {
//                                 label: "Neutral".into(),
//                                 value: WeightLogEntryMode::Neutral,
//                             },
//                             brass_bulma::SelectOption {
//                                 label: "Negative".into(),
//                                 value: WeightLogEntryMode::Negative,
//                             },
//                             brass_bulma::SelectOption {
//                                 label: "Positive".into(),
//                                 value: WeightLogEntryMode::Positive,
//                             },
//                         ]),
//                     }),
//                 )
//                 // Actions
//                 .and(
//                     brass_bulma::buttons().and(
//                         brass_bulma::button()
//                             .and_class("is-primary")
//                             .and("Save")
//                             .on_callback(|_: ClickEvent| (), &state.submit()),
//                     ),
//                 )
//                 .build()
//         }),
//     }
//     .render()
// }
