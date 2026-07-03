use dioxus::prelude::*;

#[component]
pub fn AutoExpandingTextarea(
    value: String,
    oninput: EventHandler<String>,
    #[props(default)] onfocus: Option<EventHandler<FocusEvent>>,
    #[props(default)] onblur: Option<EventHandler<FocusEvent>>,
    #[props(default = 4)] rows: u32,
    #[props(default)] placeholder: String,
) -> Element {
    rsx! {
        dxcomp::Textarea {
            class: "semantic-form__auto-textarea",
            value,
            rows: "{rows}",
            placeholder,
            oninput: move |event: FormEvent| oninput.call(event.value()),
            onfocus: move |event: FocusEvent| {
                if let Some(callback) = &onfocus {
                    callback.call(event);
                }
            },
            onblur: move |event: FocusEvent| {
                if let Some(callback) = &onblur {
                    callback.call(event);
                }
            },
        }
    }
}
