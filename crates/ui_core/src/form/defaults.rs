use std::{rc::Rc, time::Duration};

use dioxus::prelude::*;
use dxform::{FieldHandle, FormError, FormErrorSource, FormScope, ValidationStrategy};
use semantic_data::{
    schema::{NumberType, StringFormat, TemporalType, Type, TypeKind},
    value::{Date, DateTime, Time, Value},
};
use time::format_description::well_known::Rfc3339;

use crate::{
    ValueView,
    context::{use_active_scope_id, use_rpc_client},
    form::{ValueFormRenderContext, render_list_value_form, render_value_form_scope},
    ui_catalog::{RenderMode, UiCatalog, use_ui_catalog},
};

const REF_AUTOCOMPLETE_LIMIT: usize = 25;
const REF_SEARCH_FIELDS: [&str; 6] = [
    "id",
    "semantic:title",
    "title",
    "name",
    "display_name",
    "semantic:base:person:display_name",
];

pub fn register_default_form_renderers(catalog: &mut crate::UiCatalog) {
    let fallback = Rc::new(|ctx: ValueFormRenderContext| {
        rsx! {
            FallbackValueDisplay {
                scope: ctx.scope.clone(),
                value_type: ctx.value_type.clone(),
            }
        }
    });
    catalog
        .form_registry_mut()
        .set_fallback_form_renderer(fallback.clone());

    catalog
        .form_registry_mut()
        .register_type_form_renderer("string", Rc::new(render_string));
    catalog
        .form_registry_mut()
        .register_type_form_renderer("char", Rc::new(render_string));
    catalog
        .form_registry_mut()
        .register_type_form_renderer("bool", Rc::new(render_bool));
    catalog
        .form_registry_mut()
        .register_type_form_renderer("number", Rc::new(render_number));
    catalog
        .form_registry_mut()
        .register_type_form_renderer("temporal", Rc::new(render_temporal));
    catalog
        .form_registry_mut()
        .register_type_form_renderer("ref", Rc::new(render_ref));
    catalog
        .form_registry_mut()
        .register_type_form_renderer("optional", Rc::new(render_optional));
    catalog
        .form_registry_mut()
        .register_type_form_renderer("list", Rc::new(render_list));
    catalog
        .form_registry_mut()
        .register_type_form_renderer("array", Rc::new(render_list));
    catalog
        .form_registry_mut()
        .register_type_form_renderer("set", Rc::new(render_list));
    catalog
        .form_registry_mut()
        .register_type_form_renderer("class", Rc::new(render_nested_class));
    catalog
        .form_registry_mut()
        .register_type_form_renderer("record", fallback.clone());
    catalog
        .form_registry_mut()
        .register_type_form_renderer("object", fallback.clone());
}

fn render_string(ctx: ValueFormRenderContext) -> Element {
    rsx! {
        StringValueInput {
            scope: ctx.scope.clone(),
            value_type: ctx.value_type.clone(),
        }
    }
}

fn render_ref(ctx: ValueFormRenderContext) -> Element {
    rsx! {
        RefValueAutocomplete {
            scope: ctx.scope.clone(),
            value_type: ctx.value_type.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RefOption {
    id: String,
    label: String,
}

#[component]
fn RefValueAutocomplete(scope: FormScope<Value, Value>, value_type: Option<Type>) -> Element {
    let catalog = use_ui_catalog();
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let field = use_value_leaf_field(scope.clone());
    let mut query = use_signal(String::new);
    let allowed_class_ids = value_type
        .as_ref()
        .map(|ty| ref_autocomplete_class_ids(&catalog, ty))
        .unwrap_or_default();
    let selected = match field.value() {
        Value::String(value) if !value.is_empty() => Some(value),
        _ => None,
    };
    let current_id = match scope.root_value() {
        Value::Object(object) => object
            .get(semantic_data::builtin::ID_ATTRIBUTE_ID)
            .and_then(Value::as_str)
            .map(ToString::to_string),
        _ => None,
    };
    let options = use_resource(move || {
        let client = client.clone();
        let scope_id = scope_id.clone();
        let search = query();
        let allowed_class_ids = allowed_class_ids.clone();
        let current_id = current_id.clone();
        async move {
            dioxus_sdk_time::sleep(Duration::from_millis(250)).await;
            let sql = ref_autocomplete_query(&search, &allowed_class_ids, current_id.as_deref());
            let mut payload = semantic_data::value::Object::new();
            if let Some(scope_id) = scope_id {
                payload.insert("scope_id", Value::String(scope_id));
            }
            payload.insert("format", Value::String("sql".to_string()));
            payload.insert("query", Value::String(sql));
            client
                .invoke_value("semantic.db.query", Value::Object(payload))
                .await
                .map(ref_options_from_query_response)
                .unwrap_or_default()
        }
    });
    let options = options.read().clone().unwrap_or_default();
    rsx! {
        dxcomp::Combobox::<String> {
            default_value: selected,
            on_value_change: move |value| {
                if let Some(id) = value {
                    field.set_value(Value::String(id));
                }
            },
            on_query_change: move |value| query.set(value),
            placeholder: "Search entities",
            aria_label: "Referenced entity",
            list_aria_label: "Referenced entities",
            dxcomp::ComboboxEmpty { "No entity found." }
            for (index, option) in options.iter().enumerate() {
                dxcomp::ComboboxOption::<String> {
                    index,
                    value: option.id.clone(),
                    text_value: option.label.clone(),
                    span { "{option.label}" }
                    if option.label != option.id {
                        code { " {option.id}" }
                    }
                }
            }
        }
    }
}

pub fn ref_autocomplete_query(
    search: &str,
    allowed_class_ids: &[String],
    excluded_id: Option<&str>,
) -> String {
    let mut predicates = Vec::new();
    let search = search.trim();
    if !search.is_empty() {
        let pattern = format!("%{}%", escape_sql_string(search));
        let search_predicate = REF_SEARCH_FIELDS
            .iter()
            .map(|field| format!("{} ILIKE '{}'", quote_sql_ident(field), pattern))
            .collect::<Vec<_>>()
            .join(" OR ");
        predicates.push(format!("({search_predicate})"));
    }
    if !allowed_class_ids.is_empty() {
        let values = allowed_class_ids
            .iter()
            .map(|class_id| format!("'{}'", escape_sql_string(class_id)))
            .collect::<Vec<_>>()
            .join(", ");
        predicates.push(format!("{} IN ({values})", quote_sql_ident("type")));
    }
    if let Some(excluded_id) = excluded_id.filter(|id| !id.is_empty()) {
        predicates.push(format!(
            "{} <> '{}'",
            quote_sql_ident("id"),
            escape_sql_string(excluded_id)
        ));
    }

    let mut sql = "SELECT * FROM entities".to_string();
    if !predicates.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&predicates.join(" AND "));
    }
    sql.push_str(&format!(" LIMIT {REF_AUTOCOMPLETE_LIMIT}"));
    sql
}

pub fn ref_autocomplete_class_ids(catalog: &UiCatalog, ty: &Type) -> Vec<String> {
    let mut out = std::collections::BTreeSet::new();
    collect_ref_autocomplete_class_ids(catalog, ty, &mut out);
    out.into_iter().collect()
}

fn collect_ref_autocomplete_class_ids(
    catalog: &UiCatalog,
    ty: &Type,
    out: &mut std::collections::BTreeSet<String>,
) {
    match &ty.kind {
        TypeKind::Ref(type_ref) => {
            let Some(target) = catalog
                .class_by_id(&type_ref.name)
                .or_else(|| catalog.class_by_name(&type_ref.name))
            else {
                return;
            };
            out.insert(target.id.clone());
            for class in catalog.classes() {
                if class.id != target.id && ui_class_reaches(catalog, &class.id, &target.id) {
                    out.insert(class.id.clone());
                }
            }
        }
        TypeKind::Optional(optional) => {
            collect_ref_autocomplete_class_ids(catalog, &optional.inner, out);
        }
        TypeKind::Union(union) => {
            for variant in &union.variants {
                collect_ref_autocomplete_class_ids(catalog, variant, out);
            }
        }
        _ => {}
    }
}

fn ui_class_reaches(catalog: &UiCatalog, class_id: &str, target_class_id: &str) -> bool {
    fn visit(
        catalog: &UiCatalog,
        class_id: &str,
        target_class_id: &str,
        seen: &mut std::collections::BTreeSet<String>,
    ) -> bool {
        if !seen.insert(class_id.to_string()) {
            return false;
        }
        let Some(class) = catalog.class_by_id(class_id) else {
            return false;
        };
        if class.id == target_class_id {
            return true;
        }
        if let Some(parent) = &class.inherits
            && visit(catalog, &parent.id, target_class_id, seen)
        {
            return true;
        }
        class
            .extends
            .iter()
            .any(|parent| visit(catalog, &parent.id, target_class_id, seen))
    }

    visit(
        catalog,
        class_id,
        target_class_id,
        &mut std::collections::BTreeSet::new(),
    )
}

fn ref_options_from_query_response(value: Value) -> Vec<RefOption> {
    let Value::Object(object) = value else {
        return Vec::new();
    };
    let Some(Value::List(rows)) = object.get("rows") else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|row| {
            let Value::Object(object) = row else {
                return None;
            };
            let id = object.get("id").and_then(Value::as_str)?.to_string();
            let label = REF_SEARCH_FIELDS
                .iter()
                .skip(1)
                .find_map(|field| object.get(*field).and_then(Value::as_str))
                .filter(|label| !label.is_empty())
                .unwrap_or(&id)
                .to_string();
            Some(RefOption { id, label })
        })
        .collect()
}

fn quote_sql_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

fn escape_sql_string(value: &str) -> String {
    value.replace('\'', "''")
}

#[component]
fn StringValueInput(scope: FormScope<Value, Value>, value_type: Option<Type>) -> Element {
    let field = use_value_leaf_field(scope);
    let value = match field.value() {
        Value::String(value) => value,
        Value::Null | Value::Void => String::new(),
        value => crate::ui_catalog::defaults::value_to_text(&value),
    };
    let input_type = string_input_type(value_type.as_ref());
    let input_field = field.clone();
    let blur_field = field.clone();
    let focus_field = field.clone();
    rsx! {
        dxcomp::Input {
            r#type: "{input_type}",
            value,
            oninput: move |event: FormEvent| input_field.set_value(Value::String(event.value())),
            onblur: move |_event: FocusEvent| blur_field.set_focused(false),
            onfocus: move |_event: FocusEvent| focus_field.set_focused(true),
        }
    }
}

fn render_bool(ctx: ValueFormRenderContext) -> Element {
    rsx! { BoolValueInput { scope: ctx.scope } }
}

#[component]
fn BoolValueInput(scope: FormScope<Value, Value>) -> Element {
    let field = use_value_leaf_field(scope);
    let checked = matches!(field.value(), Value::Bool(true));
    rsx! {
        dxcomp::Input {
            class: "semantic-form__checkbox",
            r#type: "checkbox",
            checked,
            onchange: move |event: FormEvent| field.set_value(Value::Bool(event.checked())),
        }
    }
}

fn render_number(ctx: ValueFormRenderContext) -> Element {
    rsx! {
        NumberValueInput {
            scope: ctx.scope,
            value_type: ctx.value_type.clone(),
        }
    }
}

fn render_temporal(ctx: ValueFormRenderContext) -> Element {
    let Some(Type {
        kind: TypeKind::Temporal(temporal),
        ..
    }) = ctx.value_type.clone()
    else {
        return fallback_edit(ctx);
    };
    match temporal {
        TemporalType::Date => rsx! { DateValueInput { scope: ctx.scope } },
        TemporalType::DateTime => rsx! { DateTimeValueInput { scope: ctx.scope } },
        TemporalType::Time => rsx! { TimeValueInput { scope: ctx.scope } },
        _ => fallback_edit(ctx),
    }
}

#[component]
fn DateValueInput(scope: FormScope<Value, Value>) -> Element {
    let field = dxform::use_field(scope, || dxform::FieldSpec {
        name: "value".to_string(),
        get: Rc::new(|value: &Value| value.clone()),
        set: Rc::new(|parent: &mut Value, value: Value| *parent = value),
        format: Rc::new(date_to_string),
        parse: Rc::new(|value: &String| parse_date_value(value)),
        is_empty: Rc::new(|value: &String| value.trim().is_empty()),
        validators: Vec::new(),
        validation: ValidationStrategy::submit(),
    });
    let value = field.draft();
    rsx! {
        dxcomp::Input {
            r#type: "text",
            placeholder: "yyyy-mm-dd",
            value,
            oninput: move |event: FormEvent| field.set_draft(event.value()),
        }
    }
}

#[component]
fn DateTimeValueInput(scope: FormScope<Value, Value>) -> Element {
    let field = dxform::use_field(scope, || dxform::FieldSpec {
        name: "value".to_string(),
        get: Rc::new(|value: &Value| value.clone()),
        set: Rc::new(|parent: &mut Value, value: Value| *parent = value),
        format: Rc::new(datetime_to_string),
        parse: Rc::new(|value: &String| parse_datetime_value(value)),
        is_empty: Rc::new(|value: &String| value.trim().is_empty()),
        validators: Vec::new(),
        validation: ValidationStrategy::submit(),
    });
    let value = field.draft();
    rsx! {
        dxcomp::Input {
            r#type: "text",
            placeholder: "yyyy-mm-ddThh:mm:ssZ",
            value,
            oninput: move |event: FormEvent| field.set_draft(event.value()),
        }
    }
}

#[component]
fn TimeValueInput(scope: FormScope<Value, Value>) -> Element {
    let field = dxform::use_field(scope, || dxform::FieldSpec {
        name: "value".to_string(),
        get: Rc::new(|value: &Value| value.clone()),
        set: Rc::new(|parent: &mut Value, value: Value| *parent = value),
        format: Rc::new(time_to_string),
        parse: Rc::new(|value: &String| parse_time_value(value)),
        is_empty: Rc::new(|value: &String| value.trim().is_empty()),
        validators: Vec::new(),
        validation: ValidationStrategy::submit(),
    });
    let value = field.draft();
    rsx! {
        dxcomp::Input {
            r#type: "text",
            placeholder: "hh:mm:ss",
            value,
            oninput: move |event: FormEvent| field.set_draft(event.value()),
        }
    }
}

#[component]
fn NumberValueInput(scope: FormScope<Value, Value>, value_type: Option<Type>) -> Element {
    let ty = value_type.clone();
    let field = dxform::use_field(scope, move || {
        let parse_ty = ty.clone();
        dxform::FieldSpec {
            name: "value".to_string(),
            get: Rc::new(|value: &Value| value.clone()),
            set: Rc::new(|parent: &mut Value, value: Value| *parent = value),
            format: Rc::new(number_to_string),
            parse: Rc::new(move |value: &String| parse_number_value(value, parse_ty.as_ref())),
            is_empty: Rc::new(|value: &String| value.trim().is_empty()),
            validators: Vec::new(),
            validation: ValidationStrategy::submit(),
        }
    });
    let value = field.draft();
    rsx! {
        dxcomp::Input {
            r#type: "number",
            value,
            oninput: move |event: FormEvent| field.set_draft(event.value()),
        }
    }
}

fn render_optional(ctx: ValueFormRenderContext) -> Element {
    let Some(Type {
        kind: TypeKind::Optional(optional),
        ..
    }) = ctx.value_type.clone()
    else {
        return fallback_edit(ctx);
    };
    let inner = (*optional.inner).clone();
    rsx! {
        div { class: "semantic-form__optional",
            OptionalClearButton { scope: ctx.scope.clone() }
            {render_value_form_scope(ValueFormRenderContext {
                scope: ctx.scope,
                value_type: Some(inner),
                mode: ctx.mode,
                path: ctx.path,
            })}
        }
    }
}

#[component]
fn OptionalClearButton(scope: FormScope<Value, Value>) -> Element {
    let field = use_value_leaf_field(scope);
    rsx! {
            dxcomp::Button {
                variant: dxcomp::ButtonVariant::Outline,
                size: dxcomp::ButtonSize::Sm,
                r#type: "button",
                onclick: move |_| field.set_value(Value::Null),
                "Clear"
            }
    }
}

fn render_list(ctx: ValueFormRenderContext) -> Element {
    let Some(Type {
        kind: TypeKind::List(list),
        ..
    }) = ctx.value_type.clone()
    else {
        return fallback_edit(ctx);
    };
    render_list_value_form(ctx.scope, (*list.items).clone(), ctx.mode)
}

fn render_nested_class(ctx: ValueFormRenderContext) -> Element {
    let Some(Type {
        kind: TypeKind::Class(class),
        ..
    }) = ctx.value_type.clone()
    else {
        return fallback_edit(ctx);
    };
    let catalog = crate::use_ui_catalog();
    let class_ctx = crate::form::ClassFormRenderContext {
        form: crate::form::use_semantic_form_root(),
        scope: ctx.scope,
        class: class.clone(),
        collection: None,
        id: None,
        mode: ctx.mode,
    };
    let renderer = catalog
        .form_registry()
        .class_form_renderer(&class.id)
        .or_else(|| {
            class.inherits.as_ref().and_then(|parent| {
                catalog
                    .form_registry()
                    .class_form_renderer(&parent.id)
                    .filter(|_| catalog.class_inherits(&class.id, &parent.id))
            })
        });
    if let Some(renderer) = renderer {
        renderer(class_ctx)
    } else {
        crate::form::render_class_form_body(class_ctx)
    }
}

fn fallback_edit(ctx: ValueFormRenderContext) -> Element {
    rsx! {
        FallbackValueDisplay {
            scope: ctx.scope.clone(),
            value_type: ctx.value_type.clone(),
        }
    }
}

#[component]
fn FallbackValueDisplay(
    scope: dxform::FormScope<Value, Value>,
    value_type: Option<Type>,
) -> Element {
    rsx! {
        div { class: "semantic-form__unsupported",
            ValueView {
                value: scope.value(),
                type_hint: value_type,
                mode: RenderMode::Detail
            }
        }
    }
}

fn use_value_leaf_field(scope: FormScope<Value, Value>) -> FieldHandle<Value, Value> {
    dxform::use_field(scope, || dxform::FieldSpec {
        name: "value".to_string(),
        get: Rc::new(|value: &Value| value.clone()),
        set: Rc::new(|parent: &mut Value, value: Value| *parent = value),
        format: Rc::new(|value: &Value| value.clone()),
        parse: Rc::new(|value: &Value| Ok(value.clone())),
        is_empty: Rc::new(crate::form::is_empty_value),
        validators: Vec::new(),
        validation: ValidationStrategy::submit(),
    })
}

fn string_input_type(ty: Option<&Type>) -> &'static str {
    match ty.and_then(|ty| match &ty.kind {
        TypeKind::String(string) => string.format.as_ref(),
        _ => None,
    }) {
        Some(StringFormat::Email) => "email",
        Some(StringFormat::Uri | StringFormat::Url) => "url",
        _ => "text",
    }
}

fn number_to_string(value: &Value) -> String {
    match value {
        Value::I8(value) => value.to_string(),
        Value::I16(value) => value.to_string(),
        Value::I32(value) => value.to_string(),
        Value::I64(value) => value.to_string(),
        Value::I128(value) => value.to_string(),
        Value::U8(value) => value.to_string(),
        Value::U16(value) => value.to_string(),
        Value::U32(value) => value.to_string(),
        Value::U64(value) => value.to_string(),
        Value::U128(value) => value.to_string(),
        Value::F32(value) => value.to_string(),
        Value::F64(value) => value.to_string(),
        _ => String::new(),
    }
}

fn parse_number_value(value: &str, ty: Option<&Type>) -> std::result::Result<Value, FormError> {
    if value.trim().is_empty() {
        return Ok(Value::Null);
    }

    let number_type = ty.and_then(|ty| match &ty.kind {
        TypeKind::Number(number) => Some(number),
        _ => None,
    });
    match number_type {
        Some(NumberType::UInt(_)) | Some(NumberType::BigUInt(_)) => {
            value.parse::<u64>().map(Value::U64).map_err(parse_error)
        }
        Some(NumberType::Int(_)) | Some(NumberType::BigInt(_)) => {
            value.parse::<i64>().map(Value::I64).map_err(parse_error)
        }
        _ => value.parse::<f64>().map(Value::from).map_err(parse_error),
    }
}

fn parse_error(err: impl std::fmt::Display) -> FormError {
    FormError::new(format!("invalid number: {err}")).with_source(FormErrorSource::Parse)
}

fn date_to_string(value: &Value) -> String {
    let Value::Date(value) = value else {
        return String::new();
    };
    let value = time::Date::from(*value);
    format!(
        "{:04}-{:02}-{:02}",
        value.year(),
        u8::from(value.month()),
        value.day()
    )
}

fn parse_date_value(value: &str) -> std::result::Result<Value, FormError> {
    if value.trim().is_empty() {
        return Ok(Value::Null);
    }
    let mut parts = value.split('-');
    let year = parts
        .next()
        .ok_or_else(|| invalid_date("expected yyyy-mm-dd"))?
        .parse::<i32>()
        .map_err(invalid_date)?;
    let month = parts
        .next()
        .ok_or_else(|| invalid_date("expected yyyy-mm-dd"))?
        .parse::<u8>()
        .map_err(invalid_date)?;
    let day = parts
        .next()
        .ok_or_else(|| invalid_date("expected yyyy-mm-dd"))?
        .parse::<u8>()
        .map_err(invalid_date)?;
    if parts.next().is_some() {
        return Err(invalid_date("expected yyyy-mm-dd"));
    }
    let month = time::Month::try_from(month).map_err(invalid_date)?;
    time::Date::from_calendar_date(year, month, day)
        .map(|date| Value::Date(Date::from(date)))
        .map_err(invalid_date)
}

fn invalid_date(err: impl std::fmt::Display) -> FormError {
    FormError::new(format!("invalid date: {err}")).with_source(FormErrorSource::Parse)
}

fn datetime_to_string(value: &Value) -> String {
    let Value::DateTime(value) = value else {
        return String::new();
    };
    let value = time::OffsetDateTime::from(*value);
    value.format(&Rfc3339).unwrap_or_default()
}

fn parse_datetime_value(value: &str) -> std::result::Result<Value, FormError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(Value::Null);
    }
    time::OffsetDateTime::parse(value, &Rfc3339)
        .map(|datetime| Value::DateTime(DateTime::from(datetime)))
        .map_err(invalid_datetime)
}

fn invalid_datetime(err: impl std::fmt::Display) -> FormError {
    FormError::new(format!("invalid datetime: {err}")).with_source(FormErrorSource::Parse)
}

fn time_to_string(value: &Value) -> String {
    let Value::Time(value) = value else {
        return String::new();
    };
    let value = time::Time::from(*value);
    if value.nanosecond() == 0 {
        format!(
            "{:02}:{:02}:{:02}",
            value.hour(),
            value.minute(),
            value.second()
        )
    } else {
        format!(
            "{:02}:{:02}:{:02}.{:09}",
            value.hour(),
            value.minute(),
            value.second(),
            value.nanosecond()
        )
        .trim_end_matches('0')
        .to_string()
    }
}

fn parse_time_value(value: &str) -> std::result::Result<Value, FormError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(Value::Null);
    }
    let mut parts = value.split(':');
    let hour = parts
        .next()
        .ok_or_else(|| invalid_time("expected hh:mm[:ss[.fraction]]"))?
        .parse::<u8>()
        .map_err(invalid_time)?;
    let minute = parts
        .next()
        .ok_or_else(|| invalid_time("expected hh:mm[:ss[.fraction]]"))?
        .parse::<u8>()
        .map_err(invalid_time)?;
    let second_part = parts.next();
    if parts.next().is_some() {
        return Err(invalid_time("expected hh:mm[:ss[.fraction]]"));
    }
    let (second, nanosecond) = match second_part {
        Some(second_part) => parse_second_with_fraction(second_part)?,
        None => (0, 0),
    };
    time::Time::from_hms_nano(hour, minute, second, nanosecond)
        .map(|time| Value::Time(Time::from(time)))
        .map_err(invalid_time)
}

fn parse_second_with_fraction(value: &str) -> std::result::Result<(u8, u32), FormError> {
    let mut parts = value.split('.');
    let second = parts
        .next()
        .ok_or_else(|| invalid_time("expected seconds"))?
        .parse::<u8>()
        .map_err(invalid_time)?;
    let Some(fraction) = parts.next() else {
        return Ok((second, 0));
    };
    if parts.next().is_some() || fraction.is_empty() || fraction.len() > 9 {
        return Err(invalid_time("expected up to 9 fractional second digits"));
    }
    let nanosecond = fraction.chars().try_fold(0u32, |acc, ch| {
        ch.to_digit(10)
            .map(|digit| acc * 10 + digit)
            .ok_or_else(|| invalid_time("expected fractional second digits"))
    })? * 10u32.pow(9 - fraction.len() as u32);
    Ok((second, nanosecond))
}

fn invalid_time(err: impl std::fmt::Display) -> FormError {
    FormError::new(format!("invalid time: {err}")).with_source(FormErrorSource::Parse)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_parser_round_trips_text_format() {
        let value = parse_date_value("2025-01-01").expect("date");

        assert_eq!(date_to_string(&value), "2025-01-01");
        assert!(parse_date_value("2025-99-01").is_err());
        assert_eq!(parse_date_value("").expect("empty"), Value::Null);
    }

    #[test]
    fn number_parser_treats_empty_input_as_null() {
        let ty = Type::new(TypeKind::Number(NumberType::UInt(
            semantic_data::schema::UIntWidth::U64,
        )));

        assert_eq!(
            parse_number_value("", Some(&ty)).expect("empty"),
            Value::Null
        );
        assert_eq!(
            parse_number_value("  ", Some(&ty)).expect("blank"),
            Value::Null
        );
        assert_eq!(
            parse_number_value("42", Some(&ty)).expect("number"),
            Value::U64(42)
        );
    }

    #[test]
    fn datetime_parser_round_trips_rfc3339_text_format() {
        let value = parse_datetime_value("2025-01-01T12:34:56Z").expect("datetime");

        assert_eq!(datetime_to_string(&value), "2025-01-01T12:34:56Z");
        assert!(parse_datetime_value("2025-01-01 12:34:56").is_err());
        assert_eq!(parse_datetime_value("").expect("empty"), Value::Null);
    }

    #[test]
    fn time_parser_accepts_minute_second_and_fraction_precision() {
        let minute = parse_time_value("12:34").expect("minute");
        let second = parse_time_value("12:34:56").expect("second");
        let fraction = parse_time_value("12:34:56.123").expect("fraction");

        assert_eq!(time_to_string(&minute), "12:34:00");
        assert_eq!(time_to_string(&second), "12:34:56");
        assert_eq!(time_to_string(&fraction), "12:34:56.123");
        assert!(parse_time_value("12:99:00").is_err());
        assert_eq!(parse_time_value("").expect("empty"), Value::Null);
    }
}
