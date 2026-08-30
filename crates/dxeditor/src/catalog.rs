use std::rc::Rc;

use dioxus::prelude::*;

use crate::{
    action::{ActionRegistry, register_standard_actions},
    codec::{CodecRegistry, register_standard_codecs},
    command::{CommandRegistry, register_standard_commands},
    component::{ComponentRegistry, EditorComponentKind, EditorComponentRegistration},
    component_spec::{ComponentCatalog, ComponentCatalogError, register_standard_component_specs},
    document::{
        COMPONENT_CODE, COMPONENT_DIVIDER, COMPONENT_HEADING, COMPONENT_LINK, COMPONENT_LIST,
        COMPONENT_LIST_ITEM, COMPONENT_MENTION, COMPONENT_PARAGRAPH, COMPONENT_QUOTE,
        COMPONENT_TABLE, COMPONENT_TABLE_CELL, COMPONENT_TABLE_ROW, COMPONENT_TEXT,
    },
    format::{DocumentFormatRegistry, register_standard_document_formats},
    render::{ComponentRenderKind, EditorRenderRegistry},
    suggestion::{
        EntityMentionSuggestionProvider, InputRuleRegistry, SlashMenuSuggestionProvider,
        SuggestionRegistry,
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorCatalogConfig {
    pub register_defaults: bool,
}

impl Default for EditorCatalogConfig {
    fn default() -> Self {
        Self {
            register_defaults: true,
        }
    }
}

#[derive(Clone)]
pub struct EditorCatalog {
    codecs: CodecRegistry,
    document_formats: DocumentFormatRegistry,
    components: ComponentRegistry,
    component_specs: ComponentCatalog,
    renderers: EditorRenderRegistry,
    commands: CommandRegistry,
    actions: ActionRegistry,
    input_rules: InputRuleRegistry,
    suggestions: SuggestionRegistry,
}

impl EditorCatalog {
    pub fn builder() -> EditorCatalogBuilder {
        EditorCatalogBuilder::new()
    }

    pub fn codecs(&self) -> &CodecRegistry {
        &self.codecs
    }

    pub fn codecs_mut(&mut self) -> &mut CodecRegistry {
        &mut self.codecs
    }

    pub fn document_formats(&self) -> &DocumentFormatRegistry {
        &self.document_formats
    }

    pub fn document_formats_mut(&mut self) -> &mut DocumentFormatRegistry {
        &mut self.document_formats
    }

    pub fn components(&self) -> &ComponentRegistry {
        &self.components
    }

    pub fn components_mut(&mut self) -> &mut ComponentRegistry {
        &mut self.components
    }

    pub fn component_specs(&self) -> &ComponentCatalog {
        &self.component_specs
    }

    pub fn component_specs_mut(&mut self) -> &mut ComponentCatalog {
        &mut self.component_specs
    }

    pub fn schema_fingerprint(&self) -> String {
        self.component_specs.schema_fingerprint()
    }

    pub fn renderers(&self) -> &EditorRenderRegistry {
        &self.renderers
    }

    pub fn renderers_mut(&mut self) -> &mut EditorRenderRegistry {
        &mut self.renderers
    }

    pub fn commands(&self) -> &CommandRegistry {
        &self.commands
    }

    pub fn commands_mut(&mut self) -> &mut CommandRegistry {
        &mut self.commands
    }

    pub fn actions(&self) -> &ActionRegistry {
        &self.actions
    }

    pub fn actions_mut(&mut self) -> &mut ActionRegistry {
        &mut self.actions
    }

    pub fn input_rules(&self) -> &InputRuleRegistry {
        &self.input_rules
    }

    pub fn input_rules_mut(&mut self) -> &mut InputRuleRegistry {
        &mut self.input_rules
    }

    pub fn suggestions(&self) -> &SuggestionRegistry {
        &self.suggestions
    }

    pub fn suggestions_mut(&mut self) -> &mut SuggestionRegistry {
        &mut self.suggestions
    }
}

impl Default for EditorCatalog {
    fn default() -> Self {
        Self::builder().build()
    }
}

impl PartialEq for EditorCatalog {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

pub struct EditorCatalogBuilder {
    catalog: EditorCatalog,
    config: EditorCatalogConfig,
}

impl EditorCatalogBuilder {
    pub fn new() -> Self {
        Self {
            catalog: EditorCatalog {
                codecs: CodecRegistry::default(),
                document_formats: DocumentFormatRegistry::default(),
                components: ComponentRegistry::default(),
                component_specs: ComponentCatalog::default(),
                renderers: EditorRenderRegistry::default(),
                commands: CommandRegistry::default(),
                actions: ActionRegistry::default(),
                input_rules: InputRuleRegistry::default(),
                suggestions: SuggestionRegistry::default(),
            },
            config: EditorCatalogConfig::default(),
        }
    }

    pub fn with_config(mut self, config: EditorCatalogConfig) -> Self {
        self.config = config;
        self
    }

    pub fn build(self) -> EditorCatalog {
        self.try_build()
            .expect("standard editor catalog specifications must be valid")
    }

    pub fn try_build(mut self) -> Result<EditorCatalog, ComponentCatalogError> {
        if self.config.register_defaults {
            register_standard_codecs(&mut self.catalog.codecs);
            register_standard_document_formats(&mut self.catalog.document_formats);
            register_standard_components(&mut self.catalog.components);
            register_standard_component_specs(&mut self.catalog.component_specs)?;
            register_standard_renderers(&mut self.catalog.renderers);
            register_standard_commands(&mut self.catalog.commands);
            register_standard_actions(&mut self.catalog.actions);
            register_standard_suggestions(&mut self.catalog.suggestions);
        }
        self.catalog.component_specs.validate()?;
        Ok(self.catalog)
    }
}

impl Default for EditorCatalogBuilder {
    fn default() -> Self {
        Self::new()
    }
}

pub fn register_standard_components(registry: &mut ComponentRegistry) {
    for (id, label, kind) in [
        (COMPONENT_PARAGRAPH, "Paragraph", EditorComponentKind::Block),
        (COMPONENT_HEADING, "Heading", EditorComponentKind::Block),
        (COMPONENT_QUOTE, "Quote", EditorComponentKind::Block),
        (COMPONENT_CODE, "Code", EditorComponentKind::Block),
        (COMPONENT_LIST, "List", EditorComponentKind::Block),
        (COMPONENT_LIST_ITEM, "List Item", EditorComponentKind::Block),
        (COMPONENT_DIVIDER, "Divider", EditorComponentKind::Block),
        (COMPONENT_TEXT, "Text", EditorComponentKind::Inline),
        (COMPONENT_LINK, "Link", EditorComponentKind::Mark),
        (COMPONENT_MENTION, "Mention", EditorComponentKind::Inline),
        (COMPONENT_TABLE, "Table", EditorComponentKind::Table),
        (COMPONENT_TABLE_ROW, "Table Row", EditorComponentKind::Table),
        (
            COMPONENT_TABLE_CELL,
            "Table Cell",
            EditorComponentKind::Table,
        ),
    ] {
        registry.register(EditorComponentRegistration {
            id: id.to_string(),
            label: label.to_string(),
            kind,
        });
    }
}

pub fn register_standard_renderers(registry: &mut EditorRenderRegistry) {
    registry.set_fallback_component_renderer(Rc::new(|ctx| {
        let label = ctx
            .block
            .as_ref()
            .map(|block| block.text_content())
            .or_else(|| ctx.inline.as_ref().map(|inline| inline.text.clone()))
            .unwrap_or_default();
        rsx! { span { class: "dxeditor__fallback", "{label}" } }
    }));

    for component in [
        COMPONENT_PARAGRAPH,
        COMPONENT_HEADING,
        COMPONENT_QUOTE,
        COMPONENT_CODE,
        COMPONENT_TEXT,
        COMPONENT_MENTION,
    ] {
        registry.register_component_renderer(
            component,
            Rc::new(|ctx| match ctx.kind {
                ComponentRenderKind::Block => {
                    let text = ctx
                        .block
                        .map(|block| block.text_content())
                        .unwrap_or_default();
                    rsx! { div { class: "dxeditor__block", "{text}" } }
                }
                ComponentRenderKind::Inline => {
                    let text = ctx.inline.map(|inline| inline.text).unwrap_or_default();
                    rsx! { span { class: "dxeditor__inline", "{text}" } }
                }
                ComponentRenderKind::Mark => rsx! {},
            }),
        );
    }
}

pub fn register_standard_suggestions(registry: &mut SuggestionRegistry) {
    registry.register(Rc::new(SlashMenuSuggestionProvider));
    registry.register(Rc::new(EntityMentionSuggestionProvider));
}
