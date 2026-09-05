use std::collections::BTreeSet;

use dioxus::prelude::*;
use dioxus_icons::lucide::{Plus, Search, X};
use dxform::{FormScope, use_field};
use semantic_data::{
    builtin::{ATTR_ID, ATTR_TYPE},
    schema::{
        AttributeRef, AttributeType, ClassAttribute, ClassType, Meta, StringType, Type, TypeKind,
    },
    value::{Object, Value},
};
use semantic_db_core::catalog::OBJECT_TYPE_FIELD;

use crate::{
    ValueView,
    form::{
        AttributeFormRenderContext, ClassFormRenderContext, SemanticFormActionLabels,
        SemanticFormErrors, SemanticFormMode, SemanticFormSubmit, SemanticFormSubmitFailure,
        SemanticFormSubmitOutcome, build_class_form_options, default_value_for_type,
        is_empty_value, render_value_form_scope, value_field_spec,
    },
    ui_catalog::{RenderMode, use_ui_catalog},
};

#[derive(Clone, PartialEq)]
pub struct ClassFormField {
    pub field_name: String,
    pub storage_field_name: String,
    pub attribute: AttributeType,
    pub class_attribute: ClassAttribute,
    pub declaring_class_id: String,
}

#[derive(Clone, PartialEq)]
pub struct ClassFormFieldLabel {
    pub text: String,
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassFormRenderOptions {
    show_header: bool,
    excluded_fields: BTreeSet<String>,
}

impl ClassFormRenderOptions {
    pub fn show_header(mut self, show_header: bool) -> Self {
        self.show_header = show_header;
        self
    }

    pub fn exclude_field(mut self, field_name: impl Into<String>) -> Self {
        self.excluded_fields.insert(field_name.into());
        self
    }

    fn excludes(&self, field: &ClassFormField) -> bool {
        self.excluded_fields.contains(&field.field_name)
            || self.excluded_fields.contains(&field.storage_field_name)
    }
}

impl Default for ClassFormRenderOptions {
    fn default() -> Self {
        Self {
            show_header: true,
            excluded_fields: BTreeSet::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ClassFormFieldDomIds {
    control: String,
    label: String,
    help: String,
    errors: String,
}

pub fn class_form_field_label(field: &ClassFormField) -> ClassFormFieldLabel {
    let custom_title = field
        .class_attribute
        .meta
        .title
        .clone()
        .or_else(|| field.attribute.meta.title.clone());

    match custom_title {
        Some(text) => ClassFormFieldLabel {
            text,
            title: Some(field.attribute.id.clone()),
        },
        None => ClassFormFieldLabel {
            text: field.field_name.clone(),
            title: None,
        },
    }
}

fn class_form_field_description(field: &ClassFormField) -> Option<String> {
    field
        .class_attribute
        .meta
        .description
        .clone()
        .or_else(|| field.attribute.meta.description.clone())
}

fn class_form_field_dom_ids(path: &str) -> ClassFormFieldDomIds {
    let encoded = if path.is_empty() {
        "root".to_string()
    } else {
        path.as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let base = format!("semantic-form-field-{encoded}");
    ClassFormFieldDomIds {
        control: format!("{base}-control"),
        label: format!("{base}-label"),
        help: format!("{base}-help"),
        errors: format!("{base}-errors"),
    }
}

fn class_form_field_state(field: &ClassFormField, readonly: bool) -> Option<&'static str> {
    if field.class_attribute.computed.is_some() {
        Some("Computed")
    } else if readonly {
        Some("Read only")
    } else {
        None
    }
}

fn field_errors(meta: &dxform::FieldMeta) -> Vec<dxform::FormError> {
    let mut errors = meta.errors.clone();
    errors.extend(meta.submit_errors.clone());
    errors
}

fn described_by(ids: &ClassFormFieldDomIds, has_help: bool, has_errors: bool) -> Option<String> {
    let mut relationships = Vec::new();
    if has_help {
        relationships.push(ids.help.as_str());
    }
    if has_errors {
        relationships.push(ids.errors.as_str());
    }
    (!relationships.is_empty()).then(|| relationships.join(" "))
}

#[component]
pub fn DynamicClassForm(
    class: ClassType,
    object: Object,
    mode: SemanticFormMode,
    collection: Option<String>,
    id: Option<String>,
    scope_id: Option<String>,
    submit: Option<SemanticFormSubmit>,
    #[props(default)] action_labels: SemanticFormActionLabels,
    #[props(default)] on_dirty_change: Option<EventHandler<bool>>,
    #[props(default)] on_submitting_change: Option<EventHandler<bool>>,
    #[props(default)] on_submit_success: Option<EventHandler<SemanticFormSubmitOutcome>>,
    #[props(default)] on_submit_failure: Option<EventHandler<SemanticFormSubmitFailure>>,
) -> Element {
    let options = build_class_form_options(class, object, mode, collection, id, scope_id, submit);
    rsx! {
        crate::form::DynamicValueForm {
            options,
            action_labels,
            on_dirty_change,
            on_submitting_change,
            on_submit_success,
            on_submit_failure,
        }
    }
}

pub fn default_class_form_renderer(ctx: ClassFormRenderContext) -> Element {
    render_class_form_body(ctx)
}

pub fn render_class_form_body(ctx: ClassFormRenderContext) -> Element {
    render_class_form_body_with_options(ctx, ClassFormRenderOptions::default())
}

pub fn render_class_form_body_with_options(
    ctx: ClassFormRenderContext,
    options: ClassFormRenderOptions,
) -> Element {
    let catalog = use_ui_catalog();
    let mut fields = catalog.class_form_fields(&ctx.class);
    let primary_id_field = ctx
        .collection
        .as_deref()
        .map(|collection| primary_id_field_for_collection(&catalog, collection));
    let mut primary_field = primary_id_field.as_ref().and_then(|primary_id_field| {
        fields
            .iter()
            .position(|field| is_primary_id_form_field(field, primary_id_field))
            .map(|index| fields.remove(index))
            .or_else(|| {
                (ctx.mode == SemanticFormMode::Create)
                    .then(|| synthetic_id_form_field(primary_id_field.clone()))
            })
            .map(|field| normalize_primary_id_form_field(field, ctx.mode))
    });
    let known_field_names = fields
        .iter()
        .flat_map(|field| [field.field_name.clone(), field.storage_field_name.clone()])
        .chain(
            primary_field
                .iter()
                .flat_map(|field| [field.field_name.clone(), field.storage_field_name.clone()]),
        )
        .chain([ATTR_ID, ATTR_TYPE, OBJECT_TYPE_FIELD, "semantic:catalog:id"].map(str::to_string))
        .chain(primary_id_field)
        .chain(options.excluded_fields.iter().cloned())
        .collect::<BTreeSet<_>>();
    let extra_fields = match ctx.scope.value() {
        Value::Object(object) => object
            .iter()
            .filter(|(key, _)| !known_field_names.contains(*key))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    fields.retain(|field| !options.excludes(field));
    primary_field = primary_field.filter(|field| !options.excludes(field));
    let title = ctx
        .class
        .meta
        .title
        .clone()
        .unwrap_or_else(|| ctx.class.name.clone());
    rsx! {
        div { class: "semantic-form semantic-form--class",
            if options.show_header {
                header { class: "semantic-form__header",
                    h2 { "{title}" }
                    if let Some(id) = &ctx.id {
                        code { "{id}" }
                    }
                }
            }
            div { class: "semantic-table-wrap semantic-form__field-table-wrap",
                table { class: "semantic-field-table semantic-form__field-table",
                    tbody {
                        if let Some(field) = primary_field {
                            ClassFormFieldRow {
                                key: "{field.field_name}",
                                scope: ctx.scope.clone(),
                                class: ctx.class.clone(),
                                field,
                                mode: ctx.mode,
                                readonly: ctx.mode == SemanticFormMode::Edit,
                            }
                        }
                        for field in fields {
                            if field.field_name != OBJECT_TYPE_FIELD && field.storage_field_name != OBJECT_TYPE_FIELD {
                                {
                                    rsx! {
                                        ClassFormFieldRow {
                                            key: "{field.field_name}",
                                            scope: ctx.scope.clone(),
                                            class: ctx.class.clone(),
                                            field,
                                            mode: ctx.mode,
                                            readonly: false,
                                        }
                                    }
                                }
                            }
                        }
                        for field_name in extra_fields {
                            ExtraClassFormFieldRow {
                                key: "{field_name}",
                                scope: ctx.scope.clone(),
                                field_name,
                                mode: ctx.mode,
                                removable: !ctx.class.strict_schema,
                            }
                        }
                    }
                }
            }
            if !ctx.class.strict_schema {
                AdditionalAttributePicker {
                    scope: ctx.scope.clone(),
                    excluded: known_field_names,
                }
            }
        }
    }
}

pub(crate) fn render_class_form_field_row(
    ctx: &ClassFormRenderContext,
    field: ClassFormField,
) -> Element {
    rsx! {
        ClassFormFieldRow {
            scope: ctx.scope.clone(),
            class: ctx.class.clone(),
            field,
            mode: ctx.mode,
            readonly: false,
        }
    }
}

pub fn class_form_fields(catalog: &crate::UiCatalog, class: &ClassType) -> Vec<ClassFormField> {
    let mut fields = Vec::<ClassFormField>::new();
    collect_class_fields(catalog, class, &mut fields);
    fields.sort_by_key(|field| {
        (
            field.class_attribute.ui_order.is_none(),
            field.class_attribute.ui_order.unwrap_or(u32::MAX),
        )
    });
    fields
}

fn collect_class_fields(
    catalog: &crate::UiCatalog,
    class: &ClassType,
    fields: &mut Vec<ClassFormField>,
) {
    if let Some(parent) = &class.inherits {
        if let Some(parent_class) = catalog.class_by_id(&parent.id) {
            collect_class_fields(catalog, parent_class, fields);
        }
    }
    for extended in &class.extends {
        if let Some(extended_class) = catalog.class_by_id(&extended.id) {
            collect_class_fields(catalog, extended_class, fields);
        }
    }
    for (field_name, class_attribute) in &class.attributes {
        if let Some(attribute) = catalog.attribute_by_id(&class_attribute.attribute.id) {
            let field = ClassFormField {
                field_name: field_name.clone(),
                storage_field_name: attribute.id.clone(),
                attribute: attribute.clone(),
                class_attribute: class_attribute.clone(),
                declaring_class_id: class.id.clone(),
            };
            if let Some(existing) = fields
                .iter_mut()
                .find(|existing| existing.field_name == field_name.as_str())
            {
                *existing = field;
            } else {
                fields.push(field);
            }
        }
    }
}

#[component]
fn ClassFormFieldRow(
    scope: FormScope<Value, Value>,
    class: ClassType,
    field: ClassFormField,
    mode: SemanticFormMode,
    readonly: bool,
) -> Element {
    let catalog = use_ui_catalog();
    let label = class_form_field_label(&field);
    let description = class_form_field_description(&field);
    let field_state = class_form_field_state(&field, readonly);
    let required = field.class_attribute.required;
    let field_for_spec = field.clone();
    let catalog_for_spec = catalog.clone();
    let field_handle = use_field(scope.clone(), move || {
        crate::form::attribute_field_spec_with_storage_name(
            field_for_spec.field_name,
            field_for_spec.storage_field_name,
            field_for_spec.attribute,
            field_for_spec.class_attribute,
            catalog_for_spec,
        )
    });
    let ids = class_form_field_dom_ids(field_handle.path().as_str());
    if readonly || field.class_attribute.computed.is_some() {
        return rsx! {
            ReadonlyClassFormField {
                field: field_handle,
                label,
                description,
                field_state,
                required,
                ids,
                type_hint: field.attribute.ty.clone(),
            }
        };
    }
    let renderer = catalog
        .form_registry()
        .class_field_form_renderer(&class.id, &field.field_name)
        .or_else(|| {
            catalog
                .form_registry()
                .attribute_form_renderer(&field.attribute.id)
        });
    let body = if let Some(renderer) = renderer {
        renderer(AttributeFormRenderContext {
            scope: scope.clone(),
            field: field_handle.clone(),
            class: class.clone(),
            field_name: field.field_name.clone(),
            attribute: field.attribute.clone(),
            class_attribute: field.class_attribute.clone(),
            mode,
        })
    } else {
        let field_scope = field_handle.scope();
        render_value_form_scope(crate::form::ValueFormRenderContext {
            path: field_handle.path(),
            scope: field_scope,
            value_type: Some(field.attribute.ty.clone()),
            mode,
        })
    };
    let errors = field_errors(&field_handle.meta());
    let has_errors = !errors.is_empty();
    let aria_describedby = described_by(&ids, description.is_some(), has_errors);
    rsx! {
        tr { class: if required { "semantic-form__field semantic-form__field--required" } else { "semantic-form__field" },
            if let Some(title) = label.title.as_ref() {
                th {
                    id: ids.label.clone(),
                    scope: "row",
                    class: "semantic-form__label",
                    title: "{title}",
                    span { class: "semantic-form__label-text", "{label.text}" }
                    if let Some(field_state) = field_state {
                        span { class: "semantic-form__field-state", "{field_state}" }
                    }
                }
            } else {
                th {
                    id: ids.label.clone(),
                    scope: "row",
                    class: "semantic-form__label",
                    span { class: "semantic-form__label-text", "{label.text}" }
                    if let Some(field_state) = field_state {
                        span { class: "semantic-form__field-state", "{field_state}" }
                    }
                }
            }
            td {
                id: ids.control,
                class: "semantic-form__control",
                role: "group",
                aria_labelledby: ids.label,
                aria_describedby,
                aria_invalid: has_errors,
                {body}
                if let Some(description) = description {
                    p { id: ids.help, class: "semantic-form__help", "{description}" }
                }
                SemanticFormErrors { id: ids.errors, errors }
            }
        }
    }
}

fn primary_id_field_for_collection(catalog: &crate::UiCatalog, collection: &str) -> String {
    catalog
        .collection_by_name(collection)
        .and_then(|collection| {
            collection
                .field_ids
                .iter()
                .find(|field| field.canonical_field == ATTR_ID)
                .or_else(|| {
                    collection
                        .field_ids
                        .iter()
                        .find(|field| field.canonical_field == "semantic:catalog:id")
                })
        })
        .map(|field| field.canonical_field.clone())
        .unwrap_or_else(|| ATTR_ID.to_string())
}

fn is_primary_id_form_field(field: &ClassFormField, primary_id_field: &str) -> bool {
    field.storage_field_name == primary_id_field || field.field_name == primary_id_field
}

fn synthetic_id_form_field(primary_id_field: String) -> ClassFormField {
    let meta = Meta {
        title: Some("Id".to_string()),
        ..Meta::default()
    };
    ClassFormField {
        field_name: primary_id_field.clone(),
        storage_field_name: primary_id_field.clone(),
        attribute: AttributeType {
            id: primary_id_field.clone(),
            name: ATTR_ID.to_string(),
            ty: Type {
                kind: TypeKind::String(StringType {
                    format: None,
                    normalization: None,
                }),
                constraints: Vec::new(),
                annotations: Vec::new(),
            },
            constraints: Vec::new(),
            meta: meta.clone(),
        },
        class_attribute: ClassAttribute {
            attribute: AttributeRef {
                id: primary_id_field.clone(),
            },
            required: true,
            ui_order: Some(0),
            computed: None,
            constraints: Vec::new(),
            meta,
        },
        declaring_class_id: String::new(),
    }
}

fn normalize_primary_id_form_field(
    mut field: ClassFormField,
    mode: SemanticFormMode,
) -> ClassFormField {
    if mode == SemanticFormMode::Create {
        field.class_attribute.required = true;
    }
    if field.class_attribute.meta.title.is_none() && field.attribute.meta.title.is_none() {
        field.attribute.meta.title = Some("Id".to_string());
    }
    field
}

#[component]
fn AdditionalAttributePicker(
    scope: FormScope<Value, Value>,
    excluded: BTreeSet<String>,
) -> Element {
    let catalog = use_ui_catalog();
    let mut open = use_signal(|| false);
    let mut query = use_signal(String::new);
    let mut active = use_signal(|| 0usize);
    let mut trigger = use_signal(|| None::<std::rc::Rc<MountedData>>);
    let list_id = format!(
        "{}-attributes",
        class_form_field_dom_ids(scope.path().as_str()).control
    );
    let candidates = additional_attribute_candidates(
        catalog.attributes(),
        &excluded,
        &scope.value(),
        &query.read(),
    );
    let count = candidates.len();
    let active_index = (*active.read()).min(count.saturating_sub(1));
    let add_scope = scope.clone();
    let add = EventHandler::new(move |attribute: AttributeType| {
        add_scope.update_value(|value| {
            if let Value::Object(object) = value {
                object
                    .entry(attribute.id.clone())
                    .or_insert_with(|| default_value_for_type(&attribute.ty));
            }
        });
        query.set(String::new());
        active.set(0);
        open.set(false);
        spawn(async move {
            let mounted = trigger.peek().clone();
            if let Some(mounted) = mounted {
                let _ = mounted.set_focus(true).await;
            }
        });
    });
    let keyboard_candidates = candidates.clone();
    rsx! {
        section { class: "semantic-attribute-picker", aria_label: "Additional attributes",
            div { class: "semantic-attribute-picker__header",
                div {
                    h3 { "Additional attributes" }
                    p { "Add details beyond this class’s schema." }
                }
                button {
                    class: "semantic-attribute-picker__trigger",
                    r#type: "button",
                    aria_expanded: *open.read(),
                    aria_controls: list_id.clone(),
                    onmounted: move |event| trigger.set(Some(event.data())),
                    onclick: move |_| { open.toggle(); query.set(String::new()); active.set(0); },
                    Plus { size: "1rem" }
                    "Add attribute"
                }
            }
            if *open.read() {
                div { class: "semantic-attribute-picker__panel",
                    div { class: "semantic-attribute-picker__search",
                        Search { size: "1rem" }
                        input {
                            r#type: "search",
                            role: "combobox",
                            aria_label: "Search attributes by title or ID",
                            aria_autocomplete: "list",
                            aria_expanded: true,
                            aria_controls: list_id.clone(),
                            aria_activedescendant: (count > 0).then(|| format!("{list_id}-{active_index}")),
                            placeholder: "Search by title or ID…",
                            value: "{query}",
                            onmounted: move |event| async move { let _ = event.set_focus(true).await; },
                            oninput: move |event| { query.set(event.value()); active.set(0); },
                            onkeydown: move |event| {
                                match event.key() {
                                    Key::ArrowDown => { event.prevent_default(); active.set((active_index + 1).min(count.saturating_sub(1))); }
                                    Key::ArrowUp => { event.prevent_default(); active.set(active_index.saturating_sub(1)); }
                                    Key::Enter => {
                                        event.prevent_default();
                                        if let Some(attribute) = keyboard_candidates.get(active_index) { add.call(attribute.clone()); }
                                    }
                                    Key::Escape => {
                                        event.prevent_default();
                                        event.stop_propagation();
                                        open.set(false);
                                        spawn(async move {
                                            let mounted = trigger.peek().clone();
                                            if let Some(mounted) = mounted { let _ = mounted.set_focus(true).await; }
                                        });
                                    }
                                    _ => {}
                                }
                            },
                        }
                    }
                    div { class: "semantic-attribute-picker__results", id: list_id.clone(), role: "listbox", aria_label: "Available attributes",
                        for (index, attribute) in candidates.into_iter().enumerate() {
                            button {
                                key: "{attribute.id}",
                                id: format!("{list_id}-{index}"),
                                class: "semantic-attribute-picker__option",
                                r#type: "button",
                                role: "option",
                                tabindex: "-1",
                                aria_selected: index == active_index,
                                onmousedown: move |event| event.prevent_default(),
                                onclick: {
                                    let attribute = attribute.clone();
                                    move |_| add.call(attribute.clone())
                                },
                                div { class: "semantic-attribute-picker__option-text",
                                    span { "{attribute.meta.title.as_deref().unwrap_or(&attribute.name)}" }
                                    code { "{attribute.id}" }
                                    if let Some(description) = &attribute.meta.description {
                                        small { "{description}" }
                                    }
                                }
                                Plus { size: "1rem" }
                            }
                        }
                        if count == 0 {
                            div { class: "semantic-attribute-picker__empty",
                                if query.read().is_empty() { "All available attributes are already included." }
                                else { "No matching attributes. Try a different title or ID." }
                            }
                        }
                    }
                    div { class: "semantic-attribute-picker__hint", role: "status",
                        "{count} available · ↑ ↓ to browse · Enter to add · Esc to close"
                    }
                }
            }
        }
    }
}

fn additional_attribute_candidates<'a>(
    attributes: impl Iterator<Item = &'a AttributeType>,
    excluded: &BTreeSet<String>,
    value: &Value,
    query: &str,
) -> Vec<AttributeType> {
    let query = query.trim().to_lowercase();
    let mut attributes = attributes.filter(|attribute| {
        !excluded.contains(&attribute.id)
            && !excluded.contains(&attribute.name)
            && !matches!(attribute.id.as_str(), ATTR_ID | ATTR_TYPE | "semantic:catalog:id")
            && !matches!(value, Value::Object(object) if object.contains_key(&attribute.id) || object.contains_key(&attribute.name))
            && (query.is_empty()
                || attribute.id.to_lowercase().contains(&query)
                || attribute.name.to_lowercase().contains(&query)
                || attribute.meta.title.as_deref().unwrap_or_default().to_lowercase().contains(&query))
    }).cloned().collect::<Vec<_>>();
    attributes.sort_by_cached_key(|attribute| {
        (
            attribute
                .meta
                .title
                .as_deref()
                .unwrap_or(&attribute.name)
                .to_lowercase(),
            attribute.id.clone(),
        )
    });
    attributes
}

#[component]
fn ExtraClassFormFieldRow(
    scope: FormScope<Value, Value>,
    field_name: String,
    mode: SemanticFormMode,
    removable: bool,
) -> Element {
    let catalog = use_ui_catalog();
    let attribute = catalog
        .attribute_by_id(&field_name)
        .or_else(|| catalog.attribute_by_name(&field_name))
        .cloned();
    let label = attribute
        .as_ref()
        .and_then(|attribute| attribute.meta.title.clone())
        .unwrap_or_else(|| field_name.clone());
    let description = attribute
        .as_ref()
        .and_then(|attribute| attribute.meta.description.clone());
    let value_type = attribute.as_ref().map(|attribute| attribute.ty.clone());
    let validators = attribute
        .as_ref()
        .map(|attribute| {
            crate::form::validators_for_attribute(
                attribute,
                &ClassAttribute {
                    attribute: AttributeRef {
                        id: attribute.id.clone(),
                    },
                    required: false,
                    ui_order: None,
                    computed: None,
                    constraints: Vec::new(),
                    meta: Meta::default(),
                },
            )
        })
        .unwrap_or_default();
    let field_name_for_spec = field_name.clone();
    let field = use_field(scope.clone(), move || {
        let mut spec = value_field_spec(
            field_name_for_spec.clone(),
            Value::Null,
            std::rc::Rc::new(is_empty_value),
            validators,
            dxform::ValidationStrategy::submit(),
        );
        // Reset can remove a newly added attribute before its row unmounts.
        // Its still-registered value applier must not recreate that missing key.
        spec.set = std::rc::Rc::new(move |parent, value| {
            if let Value::Object(object) = parent {
                if object.contains_key(&field_name_for_spec) || !value.is_nullish() {
                    object.insert(field_name_for_spec.clone(), value);
                }
            }
        });
        spec
    });
    let field_scope = field.scope();
    let ids = class_form_field_dom_ids(field.path().as_str());
    let errors = field_errors(&field.meta());
    let has_errors = !errors.is_empty();
    let aria_describedby = described_by(&ids, description.is_some(), has_errors);
    rsx! {
        tr { class: "semantic-form__field semantic-form__field--extra",
            th {
                id: ids.label.clone(),
                scope: "row",
                class: "semantic-form__label",
                span { class: "semantic-form__label-text", "{label}" }
                code { class: "semantic-form__attribute-id", "{field_name}" }
                span { class: "semantic-form__field-state", if attribute.is_some() { "Additional" } else { "Unregistered" } }
            }
            td {
                id: ids.control,
                class: "semantic-form__control",
                role: "group",
                aria_labelledby: ids.label,
                aria_describedby,
                aria_invalid: has_errors,
                {render_value_form_scope(crate::form::ValueFormRenderContext {
                    path: field.path(),
                    scope: field_scope,
                    value_type,
                    mode,
                })}
                if let Some(description) = description {
                    p { id: ids.help, class: "semantic-form__help", "{description}" }
                }
                SemanticFormErrors { id: ids.errors, errors }
                if removable {
                    button {
                        class: "semantic-form__remove-attribute",
                        r#type: "button",
                        aria_label: format!("Remove {label} attribute"),
                        onclick: move |_| {
                            scope.unregister_field(&field_name);
                            scope.update_value(|value| {
                                if let Value::Object(object) = value { object.remove(&field_name); }
                            });
                        },
                        X { size: "0.875rem" }
                        "Remove attribute"
                    }
                }
            }
        }
    }
}

#[component]
fn ReadonlyClassFormField(
    field: dxform::FieldHandle<Value, Value>,
    label: ClassFormFieldLabel,
    description: Option<String>,
    field_state: Option<&'static str>,
    required: bool,
    ids: ClassFormFieldDomIds,
    type_hint: semantic_data::schema::Type,
) -> Element {
    let errors = field_errors(&field.meta());
    let has_errors = !errors.is_empty();
    let aria_describedby = described_by(&ids, description.is_some(), has_errors);
    rsx! {
        tr { class: if required { "semantic-form__field semantic-form__field--readonly semantic-form__field--required" } else { "semantic-form__field semantic-form__field--readonly" },
            if let Some(title) = label.title.as_ref() {
                th {
                    id: ids.label.clone(),
                    scope: "row",
                    class: "semantic-form__label",
                    title: "{title}",
                    span { class: "semantic-form__label-text", "{label.text}" }
                    if let Some(field_state) = field_state {
                        span { class: "semantic-form__field-state", "{field_state}" }
                    }
                }
            } else {
                th {
                    id: ids.label.clone(),
                    scope: "row",
                    class: "semantic-form__label",
                    span { class: "semantic-form__label-text", "{label.text}" }
                    if let Some(field_state) = field_state {
                        span { class: "semantic-form__field-state", "{field_state}" }
                    }
                }
            }
            td {
                id: ids.control,
                class: "semantic-form__control",
                role: "group",
                aria_labelledby: ids.label,
                aria_describedby,
                aria_invalid: has_errors,
                ValueView {
                    value: field.value(),
                    type_hint: Some(type_hint),
                    mode: RenderMode::Detail
                }
                if let Some(description) = description {
                    p { id: ids.help, class: "semantic-form__help", "{description}" }
                }
                SemanticFormErrors { id: ids.errors, errors }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn additional_attributes_filter_title_id_and_exclude_present_or_class_fields() {
        let make = |id: &str, name: &str, title: &str| {
            let mut attribute = synthetic_id_form_field(id.to_string()).attribute;
            attribute.name = name.to_string();
            attribute.meta.title = Some(title.to_string());
            attribute
        };
        let attributes = vec![
            make(ATTR_ID, ATTR_ID, "Id"),
            make(ATTR_TYPE, ATTR_TYPE, "Type"),
            make("semantic:catalog:id", "catalog_id", "Catalog ID"),
            make("test:class", "class_field", "Class field"),
            make("test:present", "present", "Present field"),
            make("test:alias", "alias", "Present alias"),
            make("test:zebra", "zebra", "Alpha title"),
            make("test:alpha", "alpha", "Zebra title"),
        ];
        let excluded = BTreeSet::from(["test:class".to_string()]);
        let value = Value::Object(Object::from_iter([
            ("test:present".to_string(), Value::Null),
            ("alias".to_string(), Value::Null),
        ]));
        let candidates = additional_attribute_candidates(attributes.iter(), &excluded, &value, "");
        assert_eq!(
            candidates
                .iter()
                .map(|attribute| attribute.id.as_str())
                .collect::<Vec<_>>(),
            ["test:zebra", "test:alpha"]
        );
        let by_title = additional_attribute_candidates(
            attributes.iter(),
            &excluded,
            &value,
            "  ALPHA TITLE  ",
        );
        assert_eq!(by_title[0].id, "test:zebra");
        let by_id =
            additional_attribute_candidates(attributes.iter(), &excluded, &value, "TEST:ALPHA");
        assert_eq!(by_id[0].id, "test:alpha");
        assert!(
            additional_attribute_candidates(attributes.iter(), &excluded, &value, "missing")
                .is_empty()
        );
    }

    #[test]
    fn field_dom_ids_are_stable_and_do_not_collapse_punctuation() {
        let colon = class_form_field_dom_ids("semantic:title");
        let dash = class_form_field_dom_ids("semantic-title");

        assert_eq!(
            colon.label,
            "semantic-form-field-73656d616e7469633a7469746c65-label"
        );
        assert_ne!(colon, dash);
        assert_eq!(
            described_by(&colon, true, true),
            Some(format!("{} {}", colon.help, colon.errors))
        );
        assert_eq!(described_by(&colon, false, false), None);
    }

    #[test]
    fn field_shell_state_only_labels_non_editable_fields() {
        let mut field = synthetic_id_form_field(ATTR_ID.to_string());
        assert_eq!(class_form_field_state(&field, false), None);

        field.class_attribute.required = false;
        assert_eq!(class_form_field_state(&field, false), None);
        assert_eq!(class_form_field_state(&field, true), Some("Read only"));
    }

    #[test]
    fn class_specific_description_overrides_attribute_help() {
        let mut field = synthetic_id_form_field(ATTR_ID.to_string());
        field.attribute.meta.description = Some("Attribute help".to_string());
        assert_eq!(
            class_form_field_description(&field).as_deref(),
            Some("Attribute help")
        );

        field.class_attribute.meta.description = Some("Class help".to_string());
        assert_eq!(
            class_form_field_description(&field).as_deref(),
            Some("Class help")
        );
    }

    #[test]
    fn class_form_options_exclude_by_field_or_storage_name() {
        let mut field = synthetic_id_form_field(ATTR_ID.to_string());
        field.field_name = "note_content".to_string();
        field.storage_field_name = "semantic:base:note:note_content".to_string();

        assert!(!ClassFormRenderOptions::default().excludes(&field));
        assert!(
            ClassFormRenderOptions::default()
                .exclude_field("note_content")
                .excludes(&field)
        );
        assert!(
            ClassFormRenderOptions::default()
                .exclude_field("semantic:base:note:note_content")
                .excludes(&field)
        );
    }
}
