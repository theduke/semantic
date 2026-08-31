use std::rc::Rc;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use dioxus::prelude::*;
use regex::RegexBuilder;
use semantic_data::schema::{NumberType, Type, TypeKind};
use semantic_ui_core::UiCatalog;
use serde::{Deserialize, Serialize};

const FILTER_STATE_PREFIX: &str = "v2:";
const MAX_FILTER_STATE_BYTES: usize = 6_000;
const MAX_ENCODED_FILTER_STATE_BYTES: usize = 8_000;
const MAX_DEPTH: usize = 5;
const MAX_NODES: usize = 50;
const MAX_LIST_VALUES: usize = 50;
const MAX_VALUE_BYTES: usize = 4_096;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredQuery {
    #[serde(default)]
    pub search: String,
    #[serde(default)]
    pub root: FilterGroup,
}

impl StructuredQuery {
    pub fn active_count(&self) -> usize {
        usize::from(!self.search.trim().is_empty()) + self.root.rule_count()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterGroup {
    #[serde(default)]
    pub combinator: GroupCombinator,
    #[serde(default)]
    pub negated: bool,
    #[serde(default)]
    pub children: Vec<FilterNode>,
}

impl Default for FilterGroup {
    fn default() -> Self {
        Self {
            combinator: GroupCombinator::All,
            negated: false,
            children: Vec::new(),
        }
    }
}

impl FilterGroup {
    fn rule_count(&self) -> usize {
        self.children
            .iter()
            .map(|child| match child {
                FilterNode::Rule(_) => 1,
                FilterNode::Group(group) => group.rule_count(),
            })
            .sum()
    }

    fn node_count(&self) -> usize {
        self.children
            .iter()
            .map(|child| match child {
                FilterNode::Rule(_) => 1,
                FilterNode::Group(group) => 1 + group.node_count(),
            })
            .sum()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupCombinator {
    #[default]
    All,
    Any,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FilterNode {
    Rule(FilterRule),
    Group(FilterGroup),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterRule {
    pub field: String,
    pub operator: FilterOperator,
    #[serde(default)]
    pub values: Vec<String>,
}

impl FilterRule {
    fn new(field: &QueryField) -> Self {
        let operator = operators_for_kind(field.kind)[0];
        Self {
            field: field.name.clone(),
            operator,
            values: if operator.value_count() == 0 {
                Vec::new()
            } else {
                vec![default_value(field.kind)]
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOperator {
    Equals,
    NotEquals,
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    Regex,
    NotRegex,
    Greater,
    GreaterOrEqual,
    Less,
    LessOrEqual,
    Between,
    In,
    IsNull,
    IsNotNull,
}

impl FilterOperator {
    fn label(self) -> &'static str {
        match self {
            Self::Equals => "equals",
            Self::NotEquals => "does not equal",
            Self::Contains => "contains",
            Self::NotContains => "does not contain",
            Self::StartsWith => "starts with",
            Self::EndsWith => "ends with",
            Self::Regex => "matches regex",
            Self::NotRegex => "does not match regex",
            Self::Greater => "is greater than",
            Self::GreaterOrEqual => "is at least",
            Self::Less => "is less than",
            Self::LessOrEqual => "is at most",
            Self::Between => "is between",
            Self::In => "is any of",
            Self::IsNull => "is missing",
            Self::IsNotNull => "is present",
        }
    }

    fn value_count(self) -> usize {
        match self {
            Self::In | Self::IsNull | Self::IsNotNull => 0,
            Self::Between => 2,
            _ => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryFieldKind {
    Text,
    SignedInteger,
    Float,
    Bool,
    PresenceOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryFieldChoice {
    pub value: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryField {
    pub name: String,
    pub label: String,
    pub description: Option<String>,
    pub kind: QueryFieldKind,
    pub deprecated: bool,
    pub choices: Vec<QueryFieldChoice>,
}

pub fn fields_for_collection(catalog: &UiCatalog, collection: &str) -> Rc<[QueryField]> {
    let mut class_choices = catalog
        .classes()
        .map(|class| QueryFieldChoice {
            value: class.id.clone(),
            label: class
                .meta
                .title
                .clone()
                .unwrap_or_else(|| class.name.clone()),
        })
        .collect::<Vec<_>>();
    class_choices.sort_by(|left, right| {
        left.label
            .to_lowercase()
            .cmp(&right.label.to_lowercase())
            .then_with(|| left.value.cmp(&right.value))
    });
    let mut fields = catalog
        .collection_by_name(collection)
        .map(|collection| {
            collection
                .field_ids
                .iter()
                .map(|stored| {
                    let name = stored.canonical_field.clone();
                    let choices = if name == "type" {
                        class_choices.clone()
                    } else {
                        Vec::new()
                    };
                    if let Some(attribute) = catalog.attribute_by_id(&name) {
                        QueryField {
                            name,
                            label: attribute
                                .meta
                                .title
                                .clone()
                                .unwrap_or_else(|| attribute.name.clone()),
                            description: attribute.meta.description.clone(),
                            kind: classify_type(&attribute.ty),
                            deprecated: attribute.meta.deprecated.is_some(),
                            choices,
                        }
                    } else {
                        let label = match name.as_str() {
                            "id" => "ID".to_string(),
                            "type" => "Type".to_string(),
                            "parent" => "Parent".to_string(),
                            _ => humanize_field(&name),
                        };
                        let kind = match name.as_str() {
                            // Type is a well-known system string. Parent is a
                            // reference and stays presence-only until typed
                            // query literals cross the RPC boundary.
                            "type" => QueryFieldKind::Text,
                            "parent" => QueryFieldKind::PresenceOnly,
                            _ => QueryFieldKind::Text,
                        };
                        QueryField {
                            name,
                            label,
                            description: None,
                            kind,
                            deprecated: false,
                            choices,
                        }
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            ["id", "type", "parent"]
                .into_iter()
                .map(|name| QueryField {
                    name: name.to_string(),
                    label: humanize_field(name),
                    description: None,
                    kind: if name == "parent" {
                        QueryFieldKind::PresenceOnly
                    } else {
                        QueryFieldKind::Text
                    },
                    deprecated: false,
                    choices: if name == "type" {
                        class_choices.clone()
                    } else {
                        Vec::new()
                    },
                })
                .collect()
        });
    fields.sort_by(|left, right| {
        let left_system = matches!(left.name.as_str(), "id" | "type" | "parent");
        let right_system = matches!(right.name.as_str(), "id" | "type" | "parent");
        right_system
            .cmp(&left_system)
            .then_with(|| left.label.to_lowercase().cmp(&right.label.to_lowercase()))
            .then_with(|| left.name.cmp(&right.name))
    });
    fields.into()
}

fn classify_type(ty: &Type) -> QueryFieldKind {
    // SQL text is parsed into semantic_data::Value before comparison. Expose
    // value operators only when that parser produces the same Value variant as
    // the dynamic form: String, Bool, I64, or F64. Unsigned, temporal,
    // UUID/IP, ref, enum, decimal, and richer values are deliberately
    // presence-only until the query boundary supports typed literals.
    match &ty.kind {
        TypeKind::Optional(optional) => classify_type(&optional.inner),
        TypeKind::Attribute(attribute) => classify_type(&attribute.ty),
        TypeKind::Number(number) => match number {
            NumberType::Int(_) | NumberType::BigInt(_) => QueryFieldKind::SignedInteger,
            NumberType::Float(_) | NumberType::Unspecified => QueryFieldKind::Float,
            _ => QueryFieldKind::PresenceOnly,
        },
        TypeKind::Bool(_) => QueryFieldKind::Bool,
        TypeKind::String(_) | TypeKind::Char(_) => QueryFieldKind::Text,
        TypeKind::Opaque(opaque) if opaque.repr.as_deref() == Some("string") => {
            QueryFieldKind::Text
        }
        _ => QueryFieldKind::PresenceOnly,
    }
}

fn humanize_field(field: &str) -> String {
    let tail = field.rsplit(':').next().unwrap_or(field);
    let mut out = tail.replace('_', " ");
    if let Some(first) = out.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    out
}

fn operators_for_kind(kind: QueryFieldKind) -> &'static [FilterOperator] {
    use FilterOperator::*;
    match kind {
        QueryFieldKind::Text => &[
            Equals,
            NotEquals,
            In,
            Contains,
            NotContains,
            StartsWith,
            EndsWith,
            Regex,
            NotRegex,
            IsNull,
            IsNotNull,
        ],
        QueryFieldKind::SignedInteger | QueryFieldKind::Float => &[
            Equals,
            NotEquals,
            In,
            Greater,
            GreaterOrEqual,
            Less,
            LessOrEqual,
            Between,
            IsNull,
            IsNotNull,
        ],
        QueryFieldKind::Bool => &[Equals, NotEquals, In, IsNull, IsNotNull],
        QueryFieldKind::PresenceOnly => &[IsNull, IsNotNull],
    }
}

fn default_value(kind: QueryFieldKind) -> String {
    if kind == QueryFieldKind::Bool {
        "true".to_string()
    } else {
        String::new()
    }
}

fn can_add_rule(root_node_count: usize, has_field: bool) -> bool {
    has_field && root_node_count < MAX_NODES
}

fn can_add_group(depth: usize, root_node_count: usize, has_field: bool) -> bool {
    has_field && depth < MAX_DEPTH && root_node_count < MAX_NODES
}

fn empty_group(combinator: GroupCombinator) -> FilterGroup {
    FilterGroup {
        combinator,
        ..FilterGroup::default()
    }
}

pub fn encode_structured_query(query: &StructuredQuery) -> std::result::Result<String, String> {
    let json = serde_json::to_string(query).map_err(|error| error.to_string())?;
    if json.len() > MAX_FILTER_STATE_BYTES {
        return Err("This filter is too large to store in a portable URL.".to_string());
    }
    Ok(format!(
        "{FILTER_STATE_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(json)
    ))
}

pub fn decode_structured_query(value: &str) -> std::result::Result<StructuredQuery, String> {
    let encoded = value
        .strip_prefix(FILTER_STATE_PREFIX)
        .ok_or_else(|| "This filter link uses an unsupported format.".to_string())?;
    if encoded.len() > MAX_ENCODED_FILTER_STATE_BYTES {
        return Err("This filter link is too large.".to_string());
    }
    let json = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| "This filter link is invalid.".to_string())?;
    if json.len() > MAX_FILTER_STATE_BYTES {
        return Err("This filter link is too large.".to_string());
    }
    serde_json::from_slice(&json).map_err(|_| "This filter link is invalid.".to_string())
}

pub fn compile_structured_predicate(
    query: &StructuredQuery,
    fields: &[QueryField],
    alias: Option<&str>,
    search_fields: &[&str],
) -> std::result::Result<Option<String>, String> {
    validate_complexity(&query.root, 0)?;
    let mut parts = Vec::new();
    let search = query.search.trim();
    if !search.is_empty() {
        if search.len() > MAX_VALUE_BYTES {
            return Err("Search is too long.".to_string());
        }
        let pattern = regex::escape(search);
        let mut predicates = Vec::new();
        for requested in search_fields {
            if let Some(field) = resolve_search_field(fields, requested) {
                let predicate = format!(
                    "{} ~* {}",
                    qualified_ident(alias, &field.name),
                    sql_string(&pattern)
                );
                if !predicates.contains(&predicate) {
                    predicates.push(predicate);
                }
            }
        }
        if predicates.is_empty() {
            return Err("The selected collection has no searchable title field.".to_string());
        }
        parts.push(format!("({})", predicates.join(" OR ")));
    }
    if let Some(group) = compile_group(&query.root, fields, alias, true)? {
        parts.push(group);
    }
    Ok((!parts.is_empty()).then(|| parts.join(" AND ")))
}

fn resolve_search_field<'a>(fields: &'a [QueryField], requested: &str) -> Option<&'a QueryField> {
    fields
        .iter()
        .find(|field| field.name == requested)
        .or_else(|| {
            fields.iter().find(|field| {
                field.name.rsplit(':').next() == Some(requested)
                    || (requested == "title" && field.name.ends_with(":title"))
            })
        })
}

fn validate_complexity(group: &FilterGroup, depth: usize) -> std::result::Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("Filters may be nested at most {MAX_DEPTH} levels."));
    }
    if group.node_count() > MAX_NODES {
        return Err(format!(
            "Filters may contain at most {MAX_NODES} conditions and groups."
        ));
    }
    for child in &group.children {
        if let FilterNode::Group(group) = child {
            validate_complexity(group, depth + 1)?;
        }
    }
    Ok(())
}

fn compile_group(
    group: &FilterGroup,
    fields: &[QueryField],
    alias: Option<&str>,
    root: bool,
) -> std::result::Result<Option<String>, String> {
    if group.children.is_empty() {
        return if root {
            Ok(None)
        } else {
            Err("Nested groups must contain at least one condition.".to_string())
        };
    }
    let mut children = Vec::new();
    for child in &group.children {
        children.push(match child {
            FilterNode::Rule(rule) => compile_rule(rule, fields, alias)?,
            FilterNode::Group(group) => compile_group(group, fields, alias, false)?
                .expect("non-root empty groups are rejected"),
        });
    }
    let joiner = match group.combinator {
        GroupCombinator::All => " AND ",
        GroupCombinator::Any => " OR ",
    };
    let expression = format!("({})", children.join(joiner));
    Ok(Some(if group.negated {
        format!("NOT {expression}")
    } else {
        expression
    }))
}

fn compile_rule(
    rule: &FilterRule,
    fields: &[QueryField],
    alias: Option<&str>,
) -> std::result::Result<String, String> {
    let field = fields
        .iter()
        .find(|field| field.name == rule.field)
        .ok_or_else(|| {
            format!(
                "Field '{}' is not available in this collection.",
                rule.field
            )
        })?;
    if !operators_for_kind(field.kind).contains(&rule.operator) {
        return Err(format!(
            "Operator '{}' cannot be used with {}.",
            rule.operator.label(),
            field.label
        ));
    }
    if rule.values.len() < rule.operator.value_count() {
        return Err(format!("{} needs a value.", field.label));
    }
    let ident = qualified_ident(alias, &field.name);
    use FilterOperator::*;
    match rule.operator {
        IsNull => Ok(format!("{ident} IS NULL")),
        IsNotNull => Ok(format!("{ident} IS NOT NULL")),
        In => {
            if rule.values.is_empty() {
                return Err(format!("{} needs at least one value.", field.label));
            }
            if rule.values.len() > MAX_LIST_VALUES {
                return Err(format!(
                    "{} accepts at most {MAX_LIST_VALUES} values.",
                    field.label
                ));
            }
            let values = rule
                .values
                .iter()
                .map(|value| compile_value(value, field.kind, &field.label))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(format!("{ident} IN ({})", values.join(", ")))
        }
        Between => {
            let low = compile_value(&rule.values[0], field.kind, &field.label)?;
            let high = compile_value(&rule.values[1], field.kind, &field.label)?;
            Ok(format!("{ident} BETWEEN {low} AND {high}"))
        }
        Contains | NotContains | StartsWith | EndsWith | Regex | NotRegex => {
            let raw = checked_text(&rule.values[0], &field.label)?;
            let pattern = match rule.operator {
                Contains | NotContains => regex::escape(raw),
                StartsWith => format!("^{}", regex::escape(raw)),
                EndsWith => format!("{}$", regex::escape(raw)),
                Regex | NotRegex => {
                    RegexBuilder::new(raw)
                        .build()
                        .map_err(|error| format!("Invalid regular expression: {error}"))?;
                    raw.to_string()
                }
                _ => return Err(format!("Unsupported pattern operator for {}.", field.label)),
            };
            let operator = match rule.operator {
                Regex => "~",
                NotRegex => "!~",
                NotContains => "!~*",
                _ => "~*",
            };
            Ok(format!("{ident} {operator} {}", sql_string(&pattern)))
        }
        operator => {
            let value = compile_value(&rule.values[0], field.kind, &field.label)?;
            let operator = match operator {
                Equals => "=",
                NotEquals => "!=",
                Greater => ">",
                GreaterOrEqual => ">=",
                Less => "<",
                LessOrEqual => "<=",
                _ => {
                    return Err(format!(
                        "Unsupported comparison operator for {}.",
                        field.label
                    ));
                }
            };
            Ok(format!("{ident} {operator} {value}"))
        }
    }
}

fn compile_value(
    value: &str,
    kind: QueryFieldKind,
    label: &str,
) -> std::result::Result<String, String> {
    let value = checked_text(value, label)?;
    match kind {
        QueryFieldKind::SignedInteger => {
            let parsed = value
                .parse::<i64>()
                .map_err(|_| format!("{label} must be a valid signed integer."))?;
            Ok(parsed.to_string())
        }
        QueryFieldKind::Float => {
            let parsed = value
                .parse::<f64>()
                .map_err(|_| format!("{label} must be a valid number."))?;
            if !parsed.is_finite() {
                return Err(format!("{label} must be a finite number."));
            }
            // Integral-looking SQL tokens become Value::I64. Force an
            // unmistakably floating token so equality and ordering operate on
            // the Value::F64 produced by the dynamic form.
            let mut sql = parsed.to_string();
            if !sql.contains(['.', 'e', 'E']) {
                sql.push_str(".0");
            }
            Ok(sql)
        }
        QueryFieldKind::Bool => match value {
            "true" => Ok("TRUE".to_string()),
            "false" => Ok("FALSE".to_string()),
            _ => Err(format!("{label} must be true or false.")),
        },
        QueryFieldKind::PresenceOnly => Err(format!("{label} only supports presence checks.")),
        QueryFieldKind::Text => Ok(sql_string(value)),
    }
}

fn checked_text<'a>(value: &'a str, label: &str) -> std::result::Result<&'a str, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{label} needs a value."));
    }
    if value.len() > MAX_VALUE_BYTES {
        return Err(format!("{label} is too long."));
    }
    Ok(value)
}

pub fn sql_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

pub fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn qualified_ident(alias: Option<&str>, field: &str) -> String {
    match alias {
        Some(alias) => format!("{}.{}", sql_ident(alias), sql_ident(field)),
        None => sql_ident(field),
    }
}

#[component]
pub fn StructuredQueryBuilder(
    draft: StructuredQuery,
    fields: Rc<[QueryField]>,
    #[props(default = "Filter results".to_string())] title: String,
    #[props(default = "Add conditions, then apply them to the result set.".to_string())]
    description: String,
    #[props(default = "Search title".to_string())] search_label: String,
    #[props(default = "Searches titles using a case-insensitive literal match.".to_string())]
    search_help: String,
    #[props(default = true)] show_search: bool,
    #[props(default)] error: Option<String>,
    on_change: EventHandler<StructuredQuery>,
) -> Element {
    let search_draft = draft.clone();
    rsx! {
        section { class: "semantic-query-builder", aria_label: title.clone(),
            div { class: "semantic-query-builder__heading",
                div { h2 { "{title}" } p { "{description}" } }
                span { class: "semantic-query-builder__badge", "Visual query" }
            }
            if show_search {
                label { class: "semantic-query-builder__search",
                    span { "{search_label}" }
                    input {
                        r#type: "search",
                        value: "{draft.search}",
                        aria_describedby: "semantic-query-builder-search-help",
                        oninput: move |event: FormEvent| {
                            let mut next = search_draft.clone();
                            next.search = event.value();
                            on_change.call(next);
                        }
                    }
                    small { id: "semantic-query-builder-search-help", "{search_help}" }
                }
            }
            FilterGroupEditor {
                query: draft.clone(),
                fields,
                path: Vec::new(),
                depth: 0,
                on_change,
            }
            if let Some(error) = error {
                p { class: "semantic-query-builder__error", role: "alert", "{error}" }
            }
        }
    }
}

#[component]
fn FilterGroupEditor(
    query: StructuredQuery,
    fields: Rc<[QueryField]>,
    path: Vec<usize>,
    depth: usize,
    on_change: EventHandler<StructuredQuery>,
) -> Element {
    let mut chooser_open = use_signal(|| false);
    let group = group_at(&query.root, &path).cloned().unwrap_or_default();
    let negate_query = query.clone();
    let add_query = query.clone();
    let negate_path = path.clone();
    let add_path = path.clone();
    let first_field = fields.iter().find(|field| field.name != "type").cloned();
    let type_field = fields
        .iter()
        .find(|field| field.name == "type" && !field.choices.is_empty())
        .cloned();
    let root_node_count = query.root.node_count();
    let can_add_field = can_add_rule(root_node_count, first_field.is_some());
    let can_add_type = can_add_rule(root_node_count, type_field.is_some());
    let can_add_nested = can_add_group(
        depth,
        root_node_count,
        first_field.is_some() || type_field.is_some(),
    );
    let can_add_any = can_add_field || can_add_type || can_add_nested;
    let chooser_id = if path.is_empty() {
        "semantic-query-builder-add-root".to_string()
    } else {
        format!(
            "semantic-query-builder-add-{}",
            path.iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join("-")
        )
    };
    let group_title = if depth == 0 {
        match group.combinator {
            GroupCombinator::All => "Filters",
            GroupCombinator::Any => "Filters (OR)",
        }
    } else {
        match group.combinator {
            GroupCombinator::All => "AND group",
            GroupCombinator::Any => "OR group",
        }
    };
    let group_help = match group.combinator {
        GroupCombinator::All => "Every filter in this group must match.",
        GroupCombinator::Any => "At least one filter in this group must match.",
    };
    rsx! {
        fieldset { class: "semantic-query-builder__group", "data-depth": depth,
            legend { "{group_title}" }
            div { class: "semantic-query-builder__group-toolbar",
                small { "{group_help}" }
                label { class: "semantic-query-builder__negate",
                    input {
                        r#type: "checkbox",
                        checked: group.negated,
                        onchange: move |event: FormEvent| {
                            let mut next = negate_query.clone();
                            if let Some(group) = group_at_mut(&mut next.root, &negate_path) { group.negated = event.checked(); }
                            on_change.call(next);
                        }
                    }
                    "Exclude this group"
                }
            }
            div { class: "semantic-query-builder__children",
                if group.children.is_empty() {
                    p { class: "semantic-query-builder__empty",
                        if depth == 0 {
                            "No filters yet. Add one to narrow the results."
                        } else {
                            "This group is empty. Add a filter before applying the query."
                        }
                    }
                }
                for (index, child) in group.children.iter().enumerate() {
                    {
                        let mut child_path = path.clone();
                        child_path.push(index);
                        let remove_path = child_path.clone();
                        let remove_query = query.clone();
                        match child {
                            FilterNode::Rule(_) => rsx! {
                                FilterRuleEditor {
                                    key: "rule-{child_path:?}",
                                    query: query.clone(), fields: fields.clone(), path: child_path,
                                    on_change,
                                }
                            },
                            FilterNode::Group(_) => rsx! {
                                div { class: "semantic-query-builder__nested",
                                    FilterGroupEditor {
                                        key: "group-{child_path:?}",
                                        query: query.clone(), fields: fields.clone(), path: child_path.clone(), depth: depth + 1,
                                        on_change,
                                    }
                                    dxcomp::Button {
                                        size: dxcomp::ButtonSize::Xs, variant: dxcomp::ButtonVariant::Ghost,
                                        aria_label: "Remove nested group",
                                        onclick: move |_| {
                                            let mut next = remove_query.clone();
                                            remove_node(&mut next.root, &remove_path);
                                            on_change.call(next);
                                        },
                                        "Remove group"
                                    }
                                }
                            },
                        }
                    }
                }
            }
            div { class: "semantic-query-builder__add-actions",
                dxcomp::Button {
                    size: dxcomp::ButtonSize::Sm,
                    variant: dxcomp::ButtonVariant::Outline,
                    disabled: !can_add_any,
                    aria_expanded: chooser_open(),
                    aria_controls: chooser_id.clone(),
                    onclick: move |_| chooser_open.toggle(),
                    "+ Add filter"
                }
                if chooser_open() {
                    div { id: chooser_id, class: "semantic-query-builder__add-chooser",
                        label {
                            span { "Filter type" }
                            select {
                                value: "",
                                aria_label: "Choose filter type",
                                onchange: move |event: FormEvent| {
                                    let value = event.value();
                                    let mut next = add_query.clone();
                                    let Some(group) = group_at_mut(&mut next.root, &add_path) else {
                                        chooser_open.set(false);
                                        return;
                                    };
                                    match value.as_str() {
                                        "type" => {
                                            let Some(field) = type_field.clone() else { return; };
                                            group.children.push(FilterNode::Rule(FilterRule {
                                                field: field.name,
                                                operator: FilterOperator::In,
                                                values: Vec::new(),
                                            }));
                                        }
                                        "field" => {
                                            let Some(field) = first_field.clone() else { return; };
                                            group.children.push(FilterNode::Rule(FilterRule::new(&field)));
                                        }
                                        "and" if can_add_nested => group.children.push(FilterNode::Group(
                                            empty_group(GroupCombinator::All),
                                        )),
                                        "or" if can_add_nested => group.children.push(FilterNode::Group(
                                            empty_group(GroupCombinator::Any),
                                        )),
                                        _ => return,
                                    }
                                    chooser_open.set(false);
                                    on_change.call(next);
                                },
                                option { value: "", disabled: true, selected: true, "Choose a filter type…" }
                                option { value: "type", disabled: !can_add_type, "Type" }
                                option { value: "field", disabled: !can_add_field, "Field comparison" }
                                option { value: "and", disabled: !can_add_nested, "AND group" }
                                option { value: "or", disabled: !can_add_nested, "OR group" }
                            }
                        }
                        dxcomp::Button {
                            size: dxcomp::ButtonSize::Xs,
                            variant: dxcomp::ButtonVariant::Ghost,
                            onclick: move |_| chooser_open.set(false),
                            "Cancel"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn FilterRuleEditor(
    query: StructuredQuery,
    fields: Rc<[QueryField]>,
    path: Vec<usize>,
    on_change: EventHandler<StructuredQuery>,
) -> Element {
    let Some(rule) = rule_at(&query.root, &path).cloned() else {
        return rsx! {};
    };
    let selected_field = fields
        .iter()
        .find(|field| field.name == rule.field)
        .cloned();
    if let Some(field) = selected_field
        .as_ref()
        .filter(|field| {
            field.name == "type" && !field.choices.is_empty() && rule.operator == FilterOperator::In
        })
        .cloned()
    {
        return rsx! {
            TypeFilterEditor { query, path, field, on_change }
        };
    }
    let kind = selected_field
        .as_ref()
        .map_or(QueryFieldKind::Text, |field| field.kind);
    let field_query = query.clone();
    let operator_query = query.clone();
    let value_query = query.clone();
    let second_value_query = query.clone();
    let remove_query = query.clone();
    let field_lookup = fields.clone();
    let field_path = path.clone();
    let operator_path = path.clone();
    let bool_value_path = path.clone();
    let text_value_path = path.clone();
    let list_value_path = path.clone();
    let second_value_path = path.clone();
    let remove_path = path.clone();
    let bool_value_query = value_query.clone();
    let text_value_query = value_query.clone();
    let list_value_query = value_query.clone();
    let field_label = selected_field
        .as_ref()
        .map_or(rule.field.as_str(), |field| field.label.as_str());
    let first_value = rule.values.first().cloned().unwrap_or_default();
    let boolean_value = rule
        .values
        .first()
        .cloned()
        .unwrap_or_else(|| "true".to_string());
    let second_value = rule.values.get(1).cloned().unwrap_or_default();
    let list_values = rule.values.join("\n");
    let field_choices = selected_field
        .as_ref()
        .map(|field| field.choices.clone())
        .unwrap_or_default();
    let input_type = match kind {
        QueryFieldKind::SignedInteger | QueryFieldKind::Float => "number",
        _ => "text",
    };
    rsx! {
        div { class: "semantic-query-builder__rule",
            label {
                span { "Field" }
                select {
                    value: "{rule.field}",
                    onchange: move |event: FormEvent| {
                        let field_name = event.value();
                        let Some(field) = field_lookup.iter().find(|field| field.name == field_name) else { return; };
                        let mut next = field_query.clone();
                        if let Some(rule) = rule_at_mut(&mut next.root, &field_path) { *rule = FilterRule::new(field); }
                        on_change.call(next);
                    },
                    if selected_field.is_none() {
                        option { value: "{rule.field}", "{rule.field} (unavailable)" }
                    }
                    if selected_field.as_ref().is_some_and(|field| field.name == "type") {
                        option { value: "type", "Type (legacy comparison)" }
                    }
                    for field in fields.iter().filter(|field| field.name != "type") {
                        option { key: "{field.name}", value: "{field.name}",
                            if field.deprecated { "{field.label} (deprecated)" } else { "{field.label}" }
                        }
                    }
                }
                if let Some(field) = &selected_field {
                    small { title: field.description.clone(), "{field.name}" }
                }
            }
            label {
                span { "Operator" }
                select {
                    value: "{operator_key(rule.operator)}",
                    onchange: move |event: FormEvent| {
                        let Some(operator) = parse_operator(&event.value()) else { return; };
                        let mut next = operator_query.clone();
                        if let Some(rule) = rule_at_mut(&mut next.root, &operator_path) {
                            rule.operator = operator;
                            rule.values.resize(operator.value_count(), default_value(kind));
                        }
                        on_change.call(next);
                    },
                    for operator in operators_for_kind(kind) {
                        option { value: "{operator_key(*operator)}", "{operator.label()}" }
                    }
                }
            }
            if rule.operator == FilterOperator::In {
                if field_choices.is_empty() {
                    label { class: "semantic-query-builder__list-input",
                        span { "Values" }
                        textarea {
                            value: "{list_values}",
                            rows: "4",
                            aria_label: "Values for {field_label}, one per line",
                            placeholder: "One value per line",
                            oninput: move |event: FormEvent| {
                                let values = event
                                    .value()
                                    .lines()
                                    .map(str::trim)
                                    .filter(|value| !value.is_empty())
                                    .map(str::to_string)
                                    .collect();
                                let mut next = list_value_query.clone();
                                set_rule_values(&mut next.root, &list_value_path, values);
                                on_change.call(next);
                            }
                        }
                        small { "Enter up to {MAX_LIST_VALUES} values, one per line." }
                    }
                } else {
                    fieldset { class: "semantic-query-builder__class-picker",
                        legend { "Classes" }
                        div { class: "semantic-query-builder__class-options",
                            for choice in field_choices.iter() {
                                {
                                    let checked = rule.values.contains(&choice.value);
                                    let choice_value = choice.value.clone();
                                    let choice_query = query.clone();
                                    let choice_path = path.clone();
                                    rsx! {
                                        label { key: "{choice.value}", title: "{choice.value}",
                                            input {
                                                r#type: "checkbox",
                                                checked,
                                                onchange: move |_| {
                                                    let mut next = choice_query.clone();
                                                    toggle_rule_value(&mut next.root, &choice_path, &choice_value);
                                                    on_change.call(next);
                                                }
                                            }
                                            span { "{choice.label}" }
                                        }
                                    }
                                }
                            }
                        }
                        small { "Select one or more known classes." }
                    }
                }
            } else if rule.operator.value_count() > 0 {
                label {
                    span { if rule.operator == FilterOperator::Between { "From" } else { "Value" } }
                    if kind == QueryFieldKind::Bool {
                        select {
                            value: "{boolean_value}",
                            onchange: move |event: FormEvent| {
                                let mut next = bool_value_query.clone();
                                set_rule_value(&mut next.root, &bool_value_path, 0, event.value());
                                on_change.call(next);
                            },
                            option { value: "true", "True" }
                            option { value: "false", "False" }
                        }
                    } else if !field_choices.is_empty() {
                        select {
                            value: "{first_value}",
                            aria_label: "Value for {field_label}",
                            onchange: move |event: FormEvent| {
                                let mut next = text_value_query.clone();
                                set_rule_value(&mut next.root, &text_value_path, 0, event.value());
                                on_change.call(next);
                            },
                            option { value: "", disabled: true, "Select a class" }
                            for choice in field_choices.iter() {
                                option { key: "{choice.value}", value: "{choice.value}", "{choice.label}" }
                            }
                        }
                    } else {
                        input {
                            r#type: input_type,
                            value: "{first_value}",
                            aria_label: "Value for {field_label}",
                            oninput: move |event: FormEvent| {
                                let mut next = text_value_query.clone();
                                set_rule_value(&mut next.root, &text_value_path, 0, event.value());
                                on_change.call(next);
                            }
                        }
                    }
                }
            }
            if rule.operator == FilterOperator::Between {
                label {
                    span { "To" }
                    input {
                        r#type: input_type,
                        value: "{second_value}",
                        aria_label: "Upper value for {field_label}",
                        oninput: move |event: FormEvent| {
                            let mut next = second_value_query.clone();
                            set_rule_value(&mut next.root, &second_value_path, 1, event.value());
                            on_change.call(next);
                        }
                    }
                }
            }
            dxcomp::Button {
                class: "semantic-query-builder__remove",
                size: dxcomp::ButtonSize::Xs,
                variant: dxcomp::ButtonVariant::Ghost,
                aria_label: "Remove condition for {field_label}",
                onclick: move |_| {
                    let mut next = remove_query.clone();
                    remove_node(&mut next.root, &remove_path);
                    on_change.call(next);
                },
                "Remove"
            }
        }
    }
}

#[component]
fn TypeFilterEditor(
    query: StructuredQuery,
    path: Vec<usize>,
    field: QueryField,
    on_change: EventHandler<StructuredQuery>,
) -> Element {
    let Some(rule) = rule_at(&query.root, &path).cloned() else {
        return rsx! {};
    };
    let selected_count = rule.values.len();
    let clear_query = query.clone();
    let clear_path = path.clone();
    let remove_query = query.clone();
    let remove_path = path.clone();
    rsx! {
        div { class: "semantic-query-builder__rule semantic-query-builder__type-filter",
            div { class: "semantic-query-builder__type-heading",
                div {
                    strong { "Type" }
                    small {
                        if selected_count == 0 {
                            "Select one or more classes (up to {MAX_LIST_VALUES})."
                        } else if selected_count == 1 {
                            "1 class selected"
                        } else {
                            "{selected_count} classes selected"
                        }
                    }
                }
                div { class: "semantic-query-builder__type-actions",
                    dxcomp::Button {
                        size: dxcomp::ButtonSize::Xs,
                        variant: dxcomp::ButtonVariant::Ghost,
                        disabled: selected_count == 0,
                        onclick: move |_| {
                            let mut next = clear_query.clone();
                            set_rule_values(&mut next.root, &clear_path, Vec::new());
                            on_change.call(next);
                        },
                        "Clear"
                    }
                    dxcomp::Button {
                        size: dxcomp::ButtonSize::Xs,
                        variant: dxcomp::ButtonVariant::Ghost,
                        aria_label: "Remove type filter",
                        onclick: move |_| {
                            let mut next = remove_query.clone();
                            remove_node(&mut next.root, &remove_path);
                            on_change.call(next);
                        },
                        "Remove"
                    }
                }
            }
            fieldset { class: "semantic-query-builder__class-picker",
                legend { "Classes" }
                div { class: "semantic-query-builder__class-options",
                    for choice in field.choices.iter() {
                        {
                            let checked = rule.values.contains(&choice.value);
                            let choice_value = choice.value.clone();
                            let choice_query = query.clone();
                            let choice_path = path.clone();
                            rsx! {
                                label { key: "{choice.value}", title: "{choice.value}",
                                    input {
                                        r#type: "checkbox",
                                        checked,
                                        disabled: !checked && selected_count >= MAX_LIST_VALUES,
                                        onchange: move |_| {
                                            let mut next = choice_query.clone();
                                            toggle_rule_value(&mut next.root, &choice_path, &choice_value);
                                            on_change.call(next);
                                        }
                                    }
                                    span { "{choice.label}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn operator_key(operator: FilterOperator) -> &'static str {
    use FilterOperator::*;
    match operator {
        Equals => "eq",
        NotEquals => "ne",
        Contains => "contains",
        NotContains => "not_contains",
        StartsWith => "starts",
        EndsWith => "ends",
        Regex => "regex",
        NotRegex => "not_regex",
        Greater => "gt",
        GreaterOrEqual => "gte",
        Less => "lt",
        LessOrEqual => "lte",
        Between => "between",
        In => "in",
        IsNull => "null",
        IsNotNull => "not_null",
    }
}

fn parse_operator(value: &str) -> Option<FilterOperator> {
    use FilterOperator::*;
    Some(match value {
        "eq" => Equals,
        "ne" => NotEquals,
        "contains" => Contains,
        "not_contains" => NotContains,
        "starts" => StartsWith,
        "ends" => EndsWith,
        "regex" => Regex,
        "not_regex" => NotRegex,
        "gt" => Greater,
        "gte" => GreaterOrEqual,
        "lt" => Less,
        "lte" => LessOrEqual,
        "between" => Between,
        "in" => In,
        "null" => IsNull,
        "not_null" => IsNotNull,
        _ => return None,
    })
}

fn group_at<'a>(root: &'a FilterGroup, path: &[usize]) -> Option<&'a FilterGroup> {
    let mut group = root;
    for index in path {
        let FilterNode::Group(next) = group.children.get(*index)? else {
            return None;
        };
        group = next;
    }
    Some(group)
}

fn group_at_mut<'a>(root: &'a mut FilterGroup, path: &[usize]) -> Option<&'a mut FilterGroup> {
    let mut group = root;
    for index in path {
        let FilterNode::Group(next) = group.children.get_mut(*index)? else {
            return None;
        };
        group = next;
    }
    Some(group)
}

fn rule_at<'a>(root: &'a FilterGroup, path: &[usize]) -> Option<&'a FilterRule> {
    let (index, parent) = path.split_last()?;
    let group = group_at(root, parent)?;
    let FilterNode::Rule(rule) = group.children.get(*index)? else {
        return None;
    };
    Some(rule)
}

fn rule_at_mut<'a>(root: &'a mut FilterGroup, path: &[usize]) -> Option<&'a mut FilterRule> {
    let (index, parent) = path.split_last()?;
    let group = group_at_mut(root, parent)?;
    let FilterNode::Rule(rule) = group.children.get_mut(*index)? else {
        return None;
    };
    Some(rule)
}

fn set_rule_value(root: &mut FilterGroup, path: &[usize], index: usize, value: String) {
    if let Some(rule) = rule_at_mut(root, path) {
        rule.values.resize(index + 1, String::new());
        rule.values[index] = value;
    }
}

fn set_rule_values(root: &mut FilterGroup, path: &[usize], values: Vec<String>) {
    if let Some(rule) = rule_at_mut(root, path) {
        rule.values = values;
    }
}

fn toggle_rule_value(root: &mut FilterGroup, path: &[usize], value: &str) {
    if let Some(rule) = rule_at_mut(root, path) {
        if let Some(index) = rule.values.iter().position(|candidate| candidate == value) {
            rule.values.remove(index);
        } else {
            rule.values.push(value.to_string());
        }
    }
}

fn remove_node(root: &mut FilterGroup, path: &[usize]) {
    let Some((index, parent)) = path.split_last() else {
        return;
    };
    if let Some(group) = group_at_mut(root, parent)
        && *index < group.children.len()
    {
        group.children.remove(*index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_data::schema::{
        FloatWidth, IntWidth, NumberType, OpaqueType, TemporalType, Type, TypeKind, UIntWidth,
    };

    fn fields() -> Vec<QueryField> {
        vec![
            QueryField {
                name: "semantic:title".into(),
                label: "Title".into(),
                description: None,
                kind: QueryFieldKind::Text,
                deprecated: false,
                choices: Vec::new(),
            },
            QueryField {
                name: "score".into(),
                label: "Score".into(),
                description: None,
                kind: QueryFieldKind::SignedInteger,
                deprecated: false,
                choices: Vec::new(),
            },
        ]
    }

    #[test]
    fn nested_groups_compile_with_precedence_and_not() {
        let query = StructuredQuery {
            search: "Ada's %".into(),
            root: FilterGroup {
                combinator: GroupCombinator::Any,
                negated: true,
                children: vec![
                    FilterNode::Rule(FilterRule {
                        field: "score".into(),
                        operator: FilterOperator::Greater,
                        values: vec!["10".into()],
                    }),
                    FilterNode::Rule(FilterRule {
                        field: "semantic:title".into(),
                        operator: FilterOperator::Regex,
                        values: vec!["^A".into()],
                    }),
                ],
            },
        };
        let sql = compile_structured_predicate(&query, &fields(), Some("e"), &["title"])
            .unwrap()
            .unwrap();
        assert!(sql.contains("\"e\".\"semantic:title\" ~* 'Ada''s %'"));
        assert!(sql.contains("NOT (\"e\".\"score\" > 10 OR \"e\".\"semantic:title\" ~ '^A')"));
    }

    #[test]
    fn invalid_values_and_unavailable_fields_are_rejected() {
        let rule = |field: &str, value: &str| StructuredQuery {
            root: FilterGroup {
                children: vec![FilterNode::Rule(FilterRule {
                    field: field.into(),
                    operator: FilterOperator::Greater,
                    values: vec![value.into()],
                })],
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(
            compile_structured_predicate(&rule("score", "nope"), &fields(), None, &[]).is_err()
        );
        assert!(compile_structured_predicate(&rule("missing", "1"), &fields(), None, &[]).is_err());
    }

    #[test]
    fn classification_only_exposes_exact_sql_literal_representations() {
        let kind = |kind| classify_type(&Type::new(kind));

        assert_eq!(
            kind(TypeKind::Number(NumberType::Int(IntWidth::I64))),
            QueryFieldKind::SignedInteger
        );
        assert_eq!(
            kind(TypeKind::Number(NumberType::Float(FloatWidth::F64))),
            QueryFieldKind::Float
        );
        assert_eq!(
            kind(TypeKind::Number(NumberType::UInt(UIntWidth::U64))),
            QueryFieldKind::PresenceOnly
        );
        assert_eq!(
            kind(TypeKind::Temporal(TemporalType::Date)),
            QueryFieldKind::PresenceOnly
        );
        assert_eq!(kind(TypeKind::Uuid), QueryFieldKind::PresenceOnly);
        assert_eq!(
            kind(TypeKind::Opaque(OpaqueType {
                id: "slug".into(),
                domain: None,
                repr: Some("string".into()),
            })),
            QueryFieldKind::Text
        );
        assert_eq!(
            kind(TypeKind::Opaque(OpaqueType {
                id: "binary-id".into(),
                domain: None,
                repr: None,
            })),
            QueryFieldKind::PresenceOnly
        );
    }

    #[test]
    fn numeric_literals_preserve_dynamic_form_value_variants() {
        let numeric_fields = vec![
            QueryField {
                name: "signed".into(),
                label: "Signed".into(),
                description: None,
                kind: QueryFieldKind::SignedInteger,
                deprecated: false,
                choices: Vec::new(),
            },
            QueryField {
                name: "float".into(),
                label: "Float".into(),
                description: None,
                kind: QueryFieldKind::Float,
                deprecated: false,
                choices: Vec::new(),
            },
        ];
        let query = |field: &str, value: &str| StructuredQuery {
            root: FilterGroup {
                children: vec![FilterNode::Rule(FilterRule {
                    field: field.into(),
                    operator: FilterOperator::Equals,
                    values: vec![value.into()],
                })],
                ..Default::default()
            },
            ..Default::default()
        };

        let signed =
            compile_structured_predicate(&query("signed", "-007"), &numeric_fields, None, &[])
                .unwrap()
                .unwrap();
        let float = compile_structured_predicate(&query("float", "1"), &numeric_fields, None, &[])
            .unwrap()
            .unwrap();

        assert_eq!(signed, "(\"signed\" = -7)");
        assert_eq!(float, "(\"float\" = 1.0)");
    }

    #[test]
    fn presence_only_fields_reject_value_comparisons() {
        let fields = vec![QueryField {
            name: "uuid".into(),
            label: "UUID".into(),
            description: None,
            kind: QueryFieldKind::PresenceOnly,
            deprecated: false,
            choices: Vec::new(),
        }];
        assert_eq!(
            operators_for_kind(QueryFieldKind::PresenceOnly),
            &[FilterOperator::IsNull, FilterOperator::IsNotNull]
        );

        let query = StructuredQuery {
            root: FilterGroup {
                children: vec![FilterNode::Rule(FilterRule {
                    field: "uuid".into(),
                    operator: FilterOperator::Equals,
                    values: vec!["eb505334-306d-44ef-bbc4-1b2fb6f1009f".into()],
                })],
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(compile_structured_predicate(&query, &fields, None, &[]).is_err());
    }

    #[test]
    fn empty_groups_use_the_requested_combinator_and_root_node_budget() {
        let group = empty_group(GroupCombinator::Any);
        assert_eq!(group.node_count(), 0);
        assert_eq!(group.combinator, GroupCombinator::Any);
        assert!(group.children.is_empty());

        assert!(can_add_rule(MAX_NODES - 1, true));
        assert!(!can_add_rule(MAX_NODES, true));
        assert!(can_add_group(0, MAX_NODES - 1, true));
        assert!(!can_add_group(0, MAX_NODES, true));
        assert!(!can_add_group(0, 0, false));
        assert!(!can_add_group(MAX_DEPTH, 0, true));
    }

    #[test]
    fn in_compiles_typed_values_and_rejects_empty_lists() {
        let query = |values| StructuredQuery {
            root: FilterGroup {
                children: vec![FilterNode::Rule(FilterRule {
                    field: "score".into(),
                    operator: FilterOperator::In,
                    values,
                })],
                ..Default::default()
            },
            ..Default::default()
        };

        let sql = compile_structured_predicate(
            &query(vec!["1".into(), "2".into(), "-3".into()]),
            &fields(),
            None,
            &[],
        )
        .unwrap()
        .unwrap();
        assert_eq!(sql, "(\"score\" IN (1, 2, -3))");
        assert!(compile_structured_predicate(&query(Vec::new()), &fields(), None, &[]).is_err());
    }

    #[test]
    fn codec_is_url_safe_and_round_trips() {
        let query = StructuredQuery {
            search: "music + video / live?".into(),
            ..Default::default()
        };
        let encoded = encode_structured_query(&query).unwrap();
        assert!(encoded.starts_with(FILTER_STATE_PREFIX));
        assert!(
            encoded[FILTER_STATE_PREFIX.len()..]
                .chars()
                .all(
                    |character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
                )
        );
        assert_eq!(decode_structured_query(&encoded).unwrap(), query);
        assert!(decode_structured_query("v1:{}").is_err());
        assert!(decode_structured_query("v2:{}").is_err());
    }
}
