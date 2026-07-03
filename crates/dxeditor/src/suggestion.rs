use std::{collections::BTreeMap, rc::Rc};

use futures::future::LocalBoxFuture;
use serde_json::Value;

use crate::{
    action::ActionSurface,
    catalog::EditorCatalog,
    state::{EditorHandle, EditorState},
};

#[derive(Clone, Debug, PartialEq)]
pub struct SuggestionItem {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub data: Value,
    pub priority: i32,
}

#[derive(Clone)]
pub struct SuggestionQueryContext {
    pub query: String,
    pub state: EditorState,
    pub catalog: EditorCatalog,
}

pub trait SuggestionProvider {
    fn trigger(&self) -> char;

    fn query(&self, ctx: SuggestionQueryContext) -> LocalBoxFuture<'static, Vec<SuggestionItem>>;

    fn apply(&self, item: SuggestionItem, editor: EditorHandle);
}

#[derive(Clone, Default)]
pub struct SuggestionRegistry {
    providers: BTreeMap<char, Rc<dyn SuggestionProvider>>,
}

impl SuggestionRegistry {
    pub fn register(&mut self, provider: Rc<dyn SuggestionProvider>) {
        self.providers.insert(provider.trigger(), provider);
    }

    pub fn provider(&self, trigger: char) -> Option<Rc<dyn SuggestionProvider>> {
        self.providers.get(&trigger).cloned()
    }
}

pub type InputRule = Rc<dyn Fn(&EditorState, &str) -> bool>;

#[derive(Clone, Default)]
pub struct InputRuleRegistry {
    rules: Vec<InputRule>,
}

impl InputRuleRegistry {
    pub fn register(&mut self, rule: InputRule) {
        self.rules.push(rule);
    }

    pub fn rules(&self) -> &[InputRule] {
        &self.rules
    }
}

pub struct SlashMenuSuggestionProvider;

impl SuggestionProvider for SlashMenuSuggestionProvider {
    fn trigger(&self) -> char {
        '/'
    }

    fn query(&self, ctx: SuggestionQueryContext) -> LocalBoxFuture<'static, Vec<SuggestionItem>> {
        Box::pin(async move {
            let query = ctx.query.to_ascii_lowercase();
            ctx.catalog
                .actions()
                .actions_for_surface(ActionSurface::SlashMenu)
                .into_iter()
                .filter(|action| action.label.to_ascii_lowercase().contains(&query))
                .map(|action| SuggestionItem {
                    id: action.id,
                    label: action.label,
                    description: action.group,
                    icon: action.icon,
                    data: Value::Null,
                    priority: action.priority,
                })
                .collect()
        })
    }

    fn apply(&self, _item: SuggestionItem, _editor: EditorHandle) {}
}

pub struct EntityMentionSuggestionProvider;

impl SuggestionProvider for EntityMentionSuggestionProvider {
    fn trigger(&self) -> char {
        '@'
    }

    fn query(&self, _ctx: SuggestionQueryContext) -> LocalBoxFuture<'static, Vec<SuggestionItem>> {
        Box::pin(async { Vec::new() })
    }

    fn apply(&self, _item: SuggestionItem, _editor: EditorHandle) {}
}
