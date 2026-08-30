use serde::{Deserialize, Serialize};

use crate::{
    component_spec::{
        ComponentCatalog, ComponentKind, FORMAT_TYPED_DOCUMENT, FormatCapability, IdentityPolicy,
    },
    document_v2::{
        COMPONENT_BLOCKQUOTE, COMPONENT_BULLET_LIST, COMPONENT_CODE_BLOCK, COMPONENT_DOCUMENT,
        COMPONENT_HARD_BREAK, COMPONENT_HEADING_V2, COMPONENT_IMAGE, COMPONENT_LIST_ITEM_V2,
        COMPONENT_MENTION_V2, COMPONENT_OPAQUE_MARKDOWN_BLOCK, COMPONENT_OPAQUE_MARKDOWN_INLINE,
        COMPONENT_ORDERED_LIST, COMPONENT_PARAGRAPH_V2, COMPONENT_TABLE_CELL_V2,
        COMPONENT_TABLE_HEADER, COMPONENT_TABLE_ROW_V2, COMPONENT_TABLE_V2, COMPONENT_TASK_ITEM,
        COMPONENT_TASK_LIST, COMPONENT_TEXT_V2, COMPONENT_THEMATIC_BREAK, COMPONENT_UNKNOWN,
        MARK_BOLD_V2, MARK_CODE_V2, MARK_ITALIC_V2, MARK_LINK_V2, MARK_STRIKE,
    },
};

pub const ENGINE_MANIFEST_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditorEngineManifest {
    pub version: u32,
    pub document_catalog_fingerprint: String,
    pub clipboard_schema_fingerprint: String,
    pub format_id: String,
    pub aria_label: String,
    pub components: Vec<EngineComponentManifest>,
    pub commands: Vec<EngineCommandManifest>,
    pub features: EngineFeatureManifest,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EngineComponentManifest {
    pub id: String,
    pub kind: ComponentKind,
    pub identity: IdentityPolicy,
    pub adapter: String,
    pub format_capability: FormatCapability,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineCommandManifest {
    pub id: String,
    pub surface: String,
    pub label: String,
    pub required_component: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineFeatureManifest {
    pub table_headers: bool,
    pub table_alignment: bool,
    pub persistent_table_widths: bool,
    pub table_spans: bool,
    pub media: bool,
    pub tasks: bool,
    pub opaque_content: bool,
}

impl EditorEngineManifest {
    pub fn new(
        catalog: &ComponentCatalog,
        format_id: impl Into<String>,
        aria_label: impl Into<String>,
    ) -> Self {
        let format_id = format_id.into();
        let capability = |id: &str| {
            catalog
                .spec_by_id(id)
                .and_then(|spec| spec.formats.get(&format_id))
                .copied()
                .unwrap_or(FormatCapability::Unsupported)
        };
        let components = catalog
            .specs()
            .map(|spec| EngineComponentManifest {
                id: spec.id.0.clone(),
                kind: spec.kind,
                identity: spec.identity,
                adapter: standard_adapter(&spec.id.0)
                    .unwrap_or("missing")
                    .to_string(),
                format_capability: capability(&spec.id.0),
            })
            .collect();
        let command_specs = [
            ("bold", "text", "Bold", MARK_BOLD_V2),
            ("italic", "text", "Italic", MARK_ITALIC_V2),
            ("strike", "text", "Strikethrough", MARK_STRIKE),
            ("code", "text", "Inline code", MARK_CODE_V2),
            ("link", "text", "Link", MARK_LINK_V2),
            ("paragraph", "slash", "Text", COMPONENT_PARAGRAPH_V2),
            ("heading", "slash", "Heading", COMPONENT_HEADING_V2),
            ("blockquote", "slash", "Quote", COMPONENT_BLOCKQUOTE),
            ("codeBlock", "slash", "Code block", COMPONENT_CODE_BLOCK),
            (
                "bulletList",
                "slash",
                "Bulleted list",
                COMPONENT_BULLET_LIST,
            ),
            (
                "orderedList",
                "slash",
                "Numbered list",
                COMPONENT_ORDERED_LIST,
            ),
            ("taskList", "slash", "Task list", COMPONENT_TASK_LIST),
            (
                "horizontalRule",
                "slash",
                "Divider",
                COMPONENT_THEMATIC_BREAK,
            ),
            ("table", "slash", "Table", COMPONENT_TABLE_V2),
            ("image", "slash", "Image", COMPONENT_IMAGE),
            (
                "addRowBefore",
                "table",
                "Add row before",
                COMPONENT_TABLE_V2,
            ),
            ("addRowAfter", "table", "Add row after", COMPONENT_TABLE_V2),
            ("deleteRow", "table", "Delete row", COMPONENT_TABLE_V2),
            (
                "addColumnBefore",
                "table",
                "Add column before",
                COMPONENT_TABLE_V2,
            ),
            (
                "addColumnAfter",
                "table",
                "Add column after",
                COMPONENT_TABLE_V2,
            ),
            ("deleteColumn", "table", "Delete column", COMPONENT_TABLE_V2),
            (
                "toggleHeaderRow",
                "table",
                "Toggle header row",
                COMPONENT_TABLE_HEADER,
            ),
            ("alignCell", "table", "Align cell", COMPONENT_TABLE_CELL_V2),
            ("deleteTable", "table", "Delete table", COMPONENT_TABLE_V2),
        ];
        let commands = command_specs
            .into_iter()
            .map(|(id, surface, label, required)| {
                let enabled = capability(required) != FormatCapability::Unsupported;
                EngineCommandManifest {
                    id: id.to_string(),
                    surface: surface.to_string(),
                    label: label.to_string(),
                    required_component: required.to_string(),
                    enabled,
                    disabled_reason: (!enabled)
                        .then(|| format!("'{required}' cannot be persisted as {format_id}")),
                }
            })
            .collect();
        let typed = format_id == FORMAT_TYPED_DOCUMENT;
        let features = EngineFeatureManifest {
            table_headers: capability(COMPONENT_TABLE_HEADER) != FormatCapability::Unsupported,
            table_alignment: capability(COMPONENT_TABLE_CELL_V2) != FormatCapability::Unsupported,
            persistent_table_widths: typed,
            table_spans: typed,
            media: capability(COMPONENT_IMAGE) != FormatCapability::Unsupported,
            tasks: capability(COMPONENT_TASK_LIST) != FormatCapability::Unsupported,
            opaque_content: capability(COMPONENT_OPAQUE_MARKDOWN_BLOCK)
                != FormatCapability::Unsupported,
        };
        Self {
            version: ENGINE_MANIFEST_VERSION,
            document_catalog_fingerprint: catalog.schema_fingerprint(),
            clipboard_schema_fingerprint: format!(
                "{}:semantic.pm-slice.v2:2026-08-30",
                catalog.schema_fingerprint()
            ),
            format_id,
            aria_label: aria_label.into(),
            components,
            commands,
            features,
        }
    }
}

fn standard_adapter(id: &str) -> Option<&'static str> {
    match id {
        COMPONENT_DOCUMENT => Some("document"),
        COMPONENT_PARAGRAPH_V2 => Some("paragraph"),
        COMPONENT_HEADING_V2 => Some("heading"),
        COMPONENT_BLOCKQUOTE => Some("blockquote"),
        COMPONENT_BULLET_LIST => Some("bulletList"),
        COMPONENT_ORDERED_LIST => Some("orderedList"),
        COMPONENT_TASK_LIST => Some("taskList"),
        COMPONENT_LIST_ITEM_V2 => Some("listItem"),
        COMPONENT_TASK_ITEM => Some("taskItem"),
        COMPONENT_CODE_BLOCK => Some("codeBlock"),
        COMPONENT_THEMATIC_BREAK => Some("horizontalRule"),
        COMPONENT_TABLE_V2 => Some("table"),
        COMPONENT_TABLE_ROW_V2 => Some("tableRow"),
        COMPONENT_TABLE_HEADER => Some("tableHeader"),
        COMPONENT_TABLE_CELL_V2 => Some("tableCell"),
        COMPONENT_TEXT_V2 => Some("text"),
        COMPONENT_HARD_BREAK => Some("hardBreak"),
        COMPONENT_IMAGE => Some("image"),
        COMPONENT_MENTION_V2 => Some("semanticMention"),
        COMPONENT_OPAQUE_MARKDOWN_BLOCK | COMPONENT_UNKNOWN => Some("opaqueBlock"),
        COMPONENT_OPAQUE_MARKDOWN_INLINE => Some("opaqueInline"),
        MARK_BOLD_V2 => Some("bold"),
        MARK_ITALIC_V2 => Some("italic"),
        MARK_STRIKE => Some("strike"),
        MARK_CODE_V2 => Some("code"),
        MARK_LINK_V2 => Some("link"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ComponentSpec, ContentRule, FORMAT_MARKDOWN};

    #[test]
    fn markdown_manifest_disables_typed_table_only_features() {
        let catalog = ComponentCatalog::standard().unwrap();
        let manifest = EditorEngineManifest::new(&catalog, FORMAT_MARKDOWN, "Editor");
        assert!(!manifest.features.persistent_table_widths);
        assert!(!manifest.features.table_spans);
        assert!(manifest.commands.iter().all(|command| command.enabled));
        assert!(
            manifest
                .components
                .iter()
                .all(|component| component.adapter != "missing")
        );
        assert_ne!(
            manifest.document_catalog_fingerprint,
            manifest.clipboard_schema_fingerprint
        );
    }

    #[test]
    fn custom_components_are_explicitly_missing_behavior_adapters() {
        let mut catalog = ComponentCatalog::standard().unwrap();
        catalog
            .register(ComponentSpec::new(
                "fixture_widget",
                "Widget",
                ComponentKind::Atom,
                Some("inline"),
                ContentRule::Empty,
                "span",
            ))
            .unwrap();
        let manifest = EditorEngineManifest::new(&catalog, FORMAT_TYPED_DOCUMENT, "Editor");
        assert_eq!(
            manifest
                .components
                .iter()
                .find(|component| component.id == "fixture_widget")
                .unwrap()
                .adapter,
            "missing"
        );
    }
}
