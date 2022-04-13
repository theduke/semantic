pub mod modal;

use std::{collections::HashSet, hash::Hash, sync::atomic::AtomicBool};

use brass::{
    dom::{
        builder::{div, p, span, tag},
        Apply, Attr, ChangeEvent, ClickEvent, Event, InputEvent, Render, Tag, TagBuilder, View,
        WithSignal,
    },
    signal::signal::{Signal, SignalExt},
    web::{create_text, elem_add_class_js, elem_remove_class_js, empty_string, set_text_data},
    DomStr,
};
use wasm_bindgen::JsCast;

use super::form::{FieldHandle, FormHandle};

pub fn bold() -> TagBuilder {
    Tag::new(Tag::B)
}

// Bulma classes
// See https://bulma.dev/classes

brass::make_str_enum! {
    Cls {
        Box = "box",
        Notification = "notification",
        Title  = "title",
        Subtitle  = "subtitle",
        Input = "input",
        TextArea = "textarea",
        Field = "field",
        Label = "label",
        Control = "control",
        Help = "help",
        Button = "button",
        Table = "table",
        Buttons = "buttons",
        IsLoading = "is-loading",
        IsGrouped = "is-grouped",
        Container = "container",
        Checkbox = "checkbox",
        Card = "card",
        CardHeader = "card-header",
        CardHeaderTitle = "card-header-title",
        CardContent = "card-content",
        Content = "content",
        Delete = "delete",
        Stub = "stub",
        Select = "select",

        File = "file",
        FileIcon = "file-icon",
        FileCta = "file-cta",
        FileLabel = "file-label",
        FileInput = "file-input",

        Icon = "icon",
        // Fontawesome.
        Fa = "fa",
        Fas = "fas",
        Far = "far",

        // Modal
        Modal = "modal",
        ModalContent = "modal-content",
        ModalBackground = "modal-background",
        ModalClose = "modal-close",

        // Tags
        Tags = "tags",
        Tag = "tag",

        // Helpers
        IsFlex = "is-flex",
        IsAlignItemsCenter = "is-align-items-center",
        IsClickable = "is-clickable",
        IsActive = "is-active",
        IsHidden = "is-hidden",
        IsStatic = "is-static",

        Mr1 = "mr-1",
        Mr2 = "mr-2",
        Mr3 = "mr-3",
        Mr4 = "mr-4",
        Mr5 = "mr-5",

    }
}

brass::make_str_enum! {
    Size {
        Large = "is-large",
    }
}

brass::make_str_enum! {
    Color {
        Primary = "is-primary",
        Link = "is-link",
        Info = "is-info",
        Success = "is-success",
        Warning = "is-warning",
        Danger = "is-danger",
    }
}

brass::make_str_enum! {
    TitleSize {
        Size1 = "is-1",
        Size2 = "is-2",
        Size3 = "is-3",
        Size4 = "is-4",
        Size5 = "is-5",
        Size6 = "is-6",
    }
}

brass::make_str_enum! {
    BtnSize {
        Small = "is-small",
        Normal = "is-normal",
        Medium = "is-medium",
        Large = "is-large",
    }
}

pub fn box_() -> TagBuilder {
    div().class(Cls::Box)
}

pub fn container() -> TagBuilder {
    div().class(Cls::Container)
}

fn notification(color: Color) -> TagBuilder {
    div().class(Cls::Notification).class(color)
}

#[inline]
pub fn notification_default() -> TagBuilder {
    div().class(Cls::Notification)
}

#[inline]
pub fn notification_warning() -> TagBuilder {
    notification(Color::Warning)
}

#[inline]
pub fn notification_error() -> TagBuilder {
    notification(Color::Danger)
}

pub fn notification_with_errors(errors: impl IntoIterator<Item = impl Apply>) -> TagBuilder {
    let mut list = tag(Tag::Ul);
    for err in errors {
        list.add_child(tag(Tag::Li).and(err));
    }

    notification_error().and(list)
}

#[inline]
pub fn notification_success() -> TagBuilder {
    notification(Color::Success)
}

pub struct NotificationBuilder {
    tag: TagBuilder,
}

impl NotificationBuilder {
    pub fn new() -> Self {
        Self {
            tag: div().class(Cls::Notification),
        }
    }

    pub fn msg<'a>(mut self, color: Color, msg: impl Into<DomStr<'a>>) -> Self {
        self.tag.add_class(color);
        self.tag.add_child(Tag::P.new().text(msg));
        self
    }

    pub fn warning<'a>(self, msg: impl Into<DomStr<'a>>) -> Self {
        self.msg(Color::Warning, msg)
    }

    pub fn error<'a>(self, msg: impl Into<DomStr<'a>>) -> Self {
        self.msg(Color::Danger, msg)
    }

    pub fn buttons(mut self, btns: ButtonGroupBuilder) -> Self {
        self.tag.add_child(btns.tag);
        self
    }

    pub fn build(self) -> TagBuilder {
        self.tag
    }
}

pub fn title(size: TitleSize) -> TagBuilder {
    TagBuilder::new(Tag::H1).class(Cls::Title).class(size)
}

#[inline]
pub fn title_1() -> TagBuilder {
    title(TitleSize::Size1)
}

#[inline]
pub fn title_2() -> TagBuilder {
    title(TitleSize::Size2)
}

pub fn subtitle(size: TitleSize) -> TagBuilder {
    TagBuilder::new(Tag::P).class(Cls::Subtitle).class(size)
}

pub fn subtitle_4() -> TagBuilder {
    subtitle(TitleSize::Size4)
}

pub fn table() -> TagBuilder {
    Tag::Table.new().class(Cls::Table)
}

pub fn field() -> TagBuilder {
    div().class(Cls::Field)
}

pub fn label() -> TagBuilder {
    tag(Tag::Label).class(Cls::Label)
}

pub fn control() -> TagBuilder {
    div().class(Cls::Control)
}

fn input() -> TagBuilder {
    tag(Tag::Input).class(Cls::Input)
}

fn textarea() -> TagBuilder {
    tag(Tag::TextArea).class(Cls::TextArea)
}

pub fn button() -> TagBuilder {
    tag(Tag::Button).class(Cls::Button)
}

pub fn button_danger() -> TagBuilder {
    button().class(Color::Danger)
}

pub fn tags() -> TagBuilder {
    div().class(Cls::Tags)
}

pub fn bulma_tag() -> TagBuilder {
    div().class(Cls::Tag)
}

pub fn tag_with_delete(content: &str, callback: impl Fn() + 'static) -> TagBuilder {
    bulma_tag()
        .and(content)
        .and(Tag::Button.new().class(BtnSize::Small).class(Cls::Delete))
        .on(move |_: ClickEvent| callback())
}

pub fn file_input(
    label: &str,
    multi: bool,
    on_change: impl Fn(ChangeEvent) + 'static,
) -> TagBuilder {
    let input = Tag::Input
        .new()
        .class(Cls::FileInput)
        .attr(Attr::Type, "file")
        .attr_toggle_if(multi, Attr::Multiple)
        .on(on_change);
    let cta = span().class(Cls::FileCta).and((
        span()
            .class(Cls::FileIcon)
            .and(Tag::I.new().class("fas").class("fa-upload")),
        span().class(Cls::FileLabel).and(label),
    ));
    let label = Tag::Label.new().class(Cls::FileLabel).and((input, cta));
    div().class(Cls::File).and(label)
}

pub struct ButtonBuilder {
    tag: TagBuilder,
}

impl ButtonBuilder {
    pub fn new() -> Self {
        Self { tag: button() }
    }

    pub fn label<'a>(mut self, label: impl Into<DomStr<'a>>) -> Self {
        self.tag.add_child(span().text(label));
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.tag.add_class(color.as_js_string());
        self
    }

    pub fn size(mut self, size: BtnSize) -> Self {
        self.tag.add_class(size);
        self
    }

    pub fn size_small(self) -> Self {
        self.size(BtnSize::Small)
    }

    pub fn size_medium(self) -> Self {
        self.size(BtnSize::Medium)
    }

    pub fn size_large(self) -> Self {
        self.size(BtnSize::Large)
    }

    pub fn static_(mut self) -> Self {
        self.tag.add_class(Cls::IsStatic);
        self
    }

    pub fn loading(mut self) -> Self {
        self.tag.add_class(Cls::IsLoading);
        self
    }

    pub fn signal_loading(mut self, s: impl Signal<Item = bool> + 'static) -> Self {
        self.tag.add_class_signal_toggle(Cls::IsLoading, s);
        self
    }

    pub fn signal_active(mut self, s: impl Signal<Item = bool> + 'static) -> Self {
        self.tag.add_class_signal_toggle(Cls::IsActive, s);
        self
    }

    pub fn signal_disabled(mut self, s: impl Signal<Item = bool> + 'static) -> Self {
        self.tag.add_attr_signal_toggle(Attr::Disabled, s);
        self
    }

    pub fn icon<'a>(mut self, icon_classes: &str) -> Self {
        self.tag.add_child(icon(icon_classes));
        self
    }

    pub fn on(mut self, f: impl Fn() + 'static) -> Self {
        self.tag.add_dom_event_listener(move |_: ClickEvent| {
            f();
        });
        self
    }

    pub fn build(self) -> TagBuilder {
        self.tag
    }
}

// pub fn button(size: Option<BtnSize>, color: Option<Color>) -> TagBuilder {
//     let mut t = TagBuilder::new(Tag::Button);
//     if let Some(size) = size {
//         t = t.class(size);
//     }
//     if let Some(color) = color {
//         t = t.class(color);
//     }
//     t
// }

pub fn buttons() -> TagBuilder {
    div().class(Cls::Buttons)
}

pub struct ButtonGroupBuilder {
    tag: TagBuilder,
}

impl ButtonGroupBuilder {
    pub fn new() -> Self {
        Self { tag: buttons() }
    }

    pub fn button<'a>(
        mut self,
        color: Color,
        label: impl Into<DomStr<'a>>,
        on_click: impl Fn() + 'static,
    ) -> Self {
        self.tag.add_child(
            ButtonBuilder::new()
                .color(color)
                .label(label)
                .on(on_click)
                .build(),
        );
        self
    }

    pub fn button_default<'a>(
        mut self,
        label: impl Into<DomStr<'a>>,
        on_click: impl Fn() + 'static,
    ) -> Self {
        self.tag
            .add_child(ButtonBuilder::new().label(label).on(on_click).build());
        self
    }

    pub fn button_warning<'a>(
        self,
        label: impl Into<DomStr<'a>>,
        on_click: impl Fn() + 'static,
    ) -> Self {
        self.button(Color::Warning, label, on_click)
    }

    pub fn button_danger<'a>(
        self,
        label: impl Into<DomStr<'a>>,
        on_click: impl Fn() + 'static,
    ) -> Self {
        self.button(Color::Danger, label, on_click)
    }
}

// Card.

pub fn card_header() -> TagBuilder {
    Tag::Header.new().class(Cls::CardHeader)
}

pub fn card_content() -> TagBuilder {
    Tag::Div.new().class(Cls::CardContent)
}

pub fn card_header_title() -> TagBuilder {
    p().class(Cls::CardHeaderTitle)
}

pub fn card() -> TagBuilder {
    p().class(Cls::Card)
}

// Icon

fn icon<'a>(classes: &str) -> TagBuilder {
    span()
        .class(Cls::Icon)
        .and(Tag::I.new().classes_raw(classes))
}

pub fn icon_fa<'a>(cls: impl Into<DomStr<'a>>) -> TagBuilder {
    let cls = cls.into();
    span()
        .class(Cls::Icon)
        .and(Tag::I.new().class(Cls::Fa).class(cls))
}

pub fn icon_fas<'a>(cls: impl Into<DomStr<'a>>) -> TagBuilder {
    span()
        .class(Cls::Icon)
        .and(Tag::I.new().class(Cls::Fas).class(cls))
}

pub fn icon_far<'a>(cls: impl Into<DomStr<'a>>) -> TagBuilder {
    span()
        .class(Cls::Icon)
        .and(Tag::I.new().class(Cls::Far).class(cls))
}

pub fn checkbox(
    label: impl Apply,
    signal: impl Signal<Item = bool> + Unpin + 'static,
    on_change: impl Fn(bool) + 'static,
) -> TagBuilder {
    tag(Tag::Label)
        .class(Cls::Checkbox)
        .and(
            tag(Tag::Input)
                .attr(Attr::Type, Cls::Checkbox)
                .attr_signal_toggle(Attr::Checked, signal)
                .on(move |ev: ChangeEvent| {
                    let checked = ev
                        .current_target()
                        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                        .map(|i| i.checked());
                    if let Some(flag) = checked {
                        on_change(flag);
                    }
                }),
        )
        .and(label)
}

pub struct InputBuilder {
    pub tag: TagBuilder,
}

impl InputBuilder {
    pub fn new() -> Self {
        Self {
            tag: tag(Tag::Input).class(Cls::Input),
        }
    }

    pub fn placeholder<'a>(mut self, value: impl Into<DomStr<'a>>) -> Self {
        self.tag.add_attr(Attr::Placeholder, value);
        self
    }

    pub fn type_<'a>(mut self, value: impl Into<DomStr<'a>>) -> Self {
        self.tag.add_attr(Attr::Type, value);
        self
    }

    pub fn size(mut self, size: BtnSize) -> Self {
        self.tag.add_class(size);
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.tag.add_class(color);
        self
    }

    pub fn value_signal(mut self, sig: impl Signal<Item = String> + Unpin + 'static) -> Self {
        self.tag.add_attr_signal(Attr::Value, sig);
        self
    }

    pub fn on_input(mut self, callback: impl Fn(String) + 'static) -> Self {
        self.tag.add_dom_event_listener(move |ev: InputEvent| {
            ev.prevent_default();
            if let Some(value) = ev.value() {
                callback(value);
            }
        });
        self
    }
}

impl Render for InputBuilder {
    fn render(self) -> View {
        self.tag.into()
    }
}

pub struct FormFieldBuilder {
    pub tag: TagBuilder,
}

impl FormFieldBuilder {
    pub fn new(label: DomStr<'_>) -> Self {
        let label = tag(Tag::Label).class(Cls::Label).text(label);
        Self {
            tag: div().class(Cls::Field).and(label),
        }
    }

    pub fn control(mut self, elem: TagBuilder) -> Self {
        self.tag.add_child(div().class(Cls::Control).and(elem));
        self
    }

    pub fn help(mut self, color: Option<Color>, content: impl Apply) -> Self {
        self.tag
            .add_child(tag(Tag::P).class_opt(color).and(content));
        self
    }

    pub fn error(self, content: impl Apply) -> Self {
        self.help(Some(Color::Danger), content)
    }
}

impl Render for FormFieldBuilder {
    fn render(self) -> View {
        self.tag.into()
    }
}

pub fn form_field<V: Clone, F: 'static>(
    name: &str,
    handle: FieldHandle<V, F>,
    mut content: TagBuilder,
) -> TagBuilder {
    let help = tag(Tag::P).class(Cls::Help);

    {
        let content_elem = content.elem().clone();
        let help_elem = help.elem().clone();
        let help_text = create_text(empty_string().into());
        help_elem.append_child(&help_text).unwrap();

        let mut have_errors = false;
        let mut have_success = false;
        let f = handle.for_each(move |status| {
            if let Err(errors) = &status.errors {
                if have_success {
                    elem_remove_class_js(&content_elem, Color::Success.as_js_string());
                    have_success = false;
                }

                if !have_errors && status.touched {
                    elem_add_class_js(&content_elem, Color::Danger.as_js_string());
                    elem_add_class_js(&help_elem, Color::Danger.as_js_string());
                    have_errors = true;
                }

                if status.touched {
                    let text = errors.join("\n");
                    set_text_data(&help_text, &text.into());
                }
            } else if have_errors {
                elem_remove_class_js(&content_elem, Color::Danger.as_js_string());
                elem_remove_class_js(&help_elem, Color::Danger.as_js_string());

                elem_add_class_js(&content_elem, Color::Success.as_js_string());
                have_success = true;
                have_errors = false;

                set_text_data(&help_text, &empty_string().into());
            }
        });
        content.register_future(f);
    }

    field()
        .and(label().and(name))
        .and(control().and((content, help)))
}

pub fn form_field_input<V: Clone>(name: &str, handle: FieldHandle<V, String>) -> TagBuilder {
    let inp = input()
        .attr_signal(Attr::Value, handle.signal_value())
        .on(handle.clone().on(|ev: InputEvent| {
            tracing::trace!("field input");
            ev.stop_immediate_propagation();
            ev.prevent_default();
            ev.value()
        }));

    form_field(name, handle, inp)
}

pub fn form_field_password<V: Clone>(name: &str, handle: FieldHandle<V, String>) -> TagBuilder {
    let inp = input()
        .attr(Attr::Type, "password")
        .attr_signal(Attr::Value, handle.signal_value())
        .on(handle.clone().on(|ev: InputEvent| {
            ev.stop_propagation();
            ev.prevent_default();
            ev.value()
        }));

    form_field(name, handle, inp)
}

/// A textarea form field.
///
/// min_rows specifies the rows="xx" attribute
/// If auto_grow is true, the area will automatically expand to the length of
/// the content.
pub fn form_field_textarea<V: Clone>(
    name: &str,
    handle: FieldHandle<V, String>,
    min_rows: usize,
    auto_grow: bool,
) -> TagBuilder {
    let area = textarea();
    let area_elem = area.elem().clone();

    let area = area
        .attr(Attr::Rows, &min_rows.to_string())
        .and(handle.get_value());

    let area = if !auto_grow {
        area.on(handle.clone().on(|e: InputEvent| {
            e.stop_propagation();
            e.value()
        }))
    } else {
        let mut current_rows = min_rows;
        area.on(handle.clone().on(move |ev: InputEvent| {
            ev.stop_propagation();
            let value = ev.value();

            let desired_rows = value
                .as_ref()
                .map(|v| v.lines().count().max(min_rows))
                .unwrap_or(min_rows);
            tracing::trace!(?desired_rows, ?current_rows);
            if current_rows != desired_rows {
                area_elem
                    .set_attribute("rows", &desired_rows.to_string())
                    .unwrap();
                current_rows = desired_rows;
            }

            value
        }))
    };

    form_field(name, handle, area)
}

pub struct SelectOption<V> {
    pub label: String,
    pub value: V,
}

pub fn form_field_select<V: Clone, F: Clone>(
    name: &str,
    options: Vec<SelectOption<F>>,
    handle: FieldHandle<V, F>,
) -> TagBuilder
where
    V: 'static,
    F: Clone + Eq + 'static,
{
    let opts = options.iter().enumerate().map(|(index, opt)| {
        Tag::Option
            .new()
            .attr(Attr::Value, index.to_string())
            .text(&opt.label)
    });

    let handle2 = handle.clone();
    let sel = Tag::Select.new().and_iter(opts).on(move |e: ChangeEvent| {
        let opt = e
            .value()
            .and_then(|v| v.parse::<usize>().ok())
            .and_then(|index| options.get(index));
        if let Some(opt) = opt {
            handle2.set(opt.value.clone());
        }
    });
    let content = div().class(Cls::Select).child(sel);

    form_field(name, handle, content)

    // let mut tags = Vec::new();

    // if options.is_empty() {
    //     tags.push(Tag::P.new().and("No options available."));
    // }

    // for option in options {
    //     let tag = span()
    //         .class(Cls::Tag)
    //         .class(Cls::IsClickable)
    //         .and(option.label);

    //     let value = option.value;
    //     let elem = tag.elem().clone();

    //     // Boxing here works around borrow checker issue that detects function
    //     // as FnMut.
    //     let handle = handle.clone();
    //     let is_active = AtomicBool::new(false);
    //     let f = move |_: ClickEvent| {
    //         if is_active.load(std::sync::atomic::Ordering::SeqCst) {
    //             handle.remove(value.clone());
    //             elem_remove_class_js(&elem, Color::Primary.as_js_string());

    //             is_active.store(false, std::sync::atomic::Ordering::SeqCst);
    //         } else {
    //             handle.add(value.clone());
    //             elem_add_class_js(&elem, Color::Primary.as_js_string());
    //             is_active.store(true, std::sync::atomic::Ordering::SeqCst);
    //         }
    //     };

    //     tags.push(tag.on(f));
    // }

    // field()
    //     .and(label().and(name))
    //     .and(control().and(div().class(Cls::Tags).and_iter(tags)))
}

pub fn form_field_tag_select<V: Clone, F>(
    name: &str,
    options: Vec<SelectOption<F>>,
    handle: FieldHandle<V, HashSet<F>>,
) -> TagBuilder
where
    V: 'static,
    F: Clone + Hash + Eq + 'static,
{
    let mut tags = Vec::new();

    if options.is_empty() {
        tags.push(Tag::P.new().and("No options available."));
    }

    for option in options {
        let tag = span()
            .class(Cls::Tag)
            .class(Cls::IsClickable)
            .and(option.label);

        let value = option.value;
        let elem = tag.elem().clone();

        // Boxing here works around borrow checker issue that detects function
        // as FnMut.
        let handle = handle.clone();
        let is_active = AtomicBool::new(false);
        let f = move |_: ClickEvent| {
            if is_active.load(std::sync::atomic::Ordering::SeqCst) {
                handle.remove(value.clone());
                elem_remove_class_js(&elem, Color::Primary.as_js_string());

                is_active.store(false, std::sync::atomic::Ordering::SeqCst);
            } else {
                handle.add(value.clone());
                elem_add_class_js(&elem, Color::Primary.as_js_string());
                is_active.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        };

        tags.push(tag.on(f));
    }

    field()
        .and(label().and(name))
        .and(control().and(div().class(Cls::Tags).and_iter(tags)))
}

pub fn form_field_checkbox<V: Clone>(name: &str, handle: FieldHandle<V, bool>) -> TagBuilder {
    let help = handle.signal_errors().map(|errors| match errors {
        Some(errors) => notification_with_errors(errors).into(),
        None => View::Empty,
    });

    field().and(control().and(checkbox(
        (name, WithSignal(help)),
        handle.signal_value(),
        move |flag| {
            tracing::trace!(?flag, "checkbox value change");
            handle.set(flag);
        },
    )))
}

pub fn form_button_submit<V: Clone>(label: &str, handle: FormHandle<V>) -> TagBuilder {
    button()
        .class(Color::Link)
        .and(label)
        .attr_signal_toggle(Attr::Disabled, handle.signal_not_submittable())
        .class_signal_toggle(Cls::IsLoading, handle.signal_loading())
        .on(move |_: ClickEvent| {
            handle.submit();
        })
}

pub fn form_button_reset_default<V: Clone + Default>(
    label: &str,
    handle: FormHandle<V>,
) -> TagBuilder {
    button()
        .and(label)
        .attr_signal_toggle(Attr::Disabled, handle.signal_loading())
        .on(move |_: ClickEvent| {
            handle.reset_default();
        })
}

pub struct FormButtonBuilder<'a, V: Clone + 'static> {
    handle: &'a FormHandle<V>,
    control: TagBuilder,
}

impl<'a, V: Clone> FormButtonBuilder<'a, V> {
    pub fn new(handle: &'a FormHandle<V>) -> Self {
        Self {
            handle,
            control: control(),
        }
    }

    pub fn submit(mut self, label: &str) -> Self {
        self.control
            .add_child(form_button_submit(label, self.handle.clone()));
        self
    }

    pub fn reset_default(mut self, label: &str) -> Self
    where
        V: Default,
    {
        self.control
            .add_child(form_button_reset_default(label, self.handle.clone()));
        self
    }

    fn build(self) -> TagBuilder {
        field().and(self.control)
    }
}

pub fn form_errors<V: Clone>(handle: &FormHandle<V>) -> TagBuilder {
    // TODO: probably want to use a MutableVec instead to avoid replacing the
    // errors.
    div()
        .style_raw("margin: 3rem 0;")
        .signal(handle.signal_status().map(|status| {
            if let Err(errors) = status.errors {
                let text = errors.join("\n");
                focus(notification_error().and(text)).into()
            } else if let Some(err) = status.submit_error {
                notification_error().and(err.to_string()).into()
            } else {
                View::Empty
            }
        }))
}

pub struct FormRenderer<V: 'static> {
    pub handle: FormHandle<V>,
    pub tag: TagBuilder,
}

impl<V: Clone + 'static> FormRenderer<V> {
    pub fn new(handle: FormHandle<V>) -> Self {
        let mut form = tag(Tag::Form);
        let handle2 = handle.clone();
        form.add_event_listener(Event::Submit, move |ev| {
            ev.prevent_default();
            ev.stop_propagation();

            handle2.submit();
        });
        Self { handle, tag: form }
    }

    pub fn field<F>(
        mut self,
        field: FieldHandle<V, F>,
        render: impl FnOnce(FieldHandle<V, F>) -> TagBuilder,
    ) -> Self {
        self.tag.add_child(render(field));
        self
    }

    pub fn and(mut self, item: impl Apply) -> Self {
        item.apply(&mut self.tag);
        self
    }

    pub fn buttons_submit(self, label: &str) -> TagBuilder {
        let handle = self.handle;

        self.tag.and((
            form_errors(&handle),
            FormButtonBuilder::new(&handle).submit(label).build(),
        ))
    }

    pub fn with_buttons(
        mut self,
        f: impl FnOnce(FormButtonBuilder<'_, V>) -> FormButtonBuilder<'_, V>,
    ) -> Self {
        self.tag
            .add_child(f(FormButtonBuilder::new(&self.handle)).build());
        self
    }

    pub fn build(self) -> TagBuilder {
        self.tag
    }
}

/// Renders the given content, and focuses on the the content as soon as it is
/// rendered.
pub fn focus(content: TagBuilder) -> TagBuilder {
    let wrapper = div().and(content);

    // Note: unwrap is ok because div is a HtmlElement.
    let elem = wrapper
        .elem()
        .clone()
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap();

    wasm_bindgen_futures::spawn_local(async move {
        if let Err(_err) = elem.focus() {
            tracing::warn!("Could not focus loader element");
        }
    });

    wrapper
}
