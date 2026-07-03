use std::{collections::BTreeMap, rc::Rc};

use serde_json::{Value, json};

use crate::{EditorError, catalog::EditorCatalog, state::EditorState};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ActionSurface {
    MainToolbar,
    FloatingToolbar,
    SlashMenu,
    ContextMenu,
    LinkPopover,
    TableToolbar,
    CommandPalette,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyBinding {
    pub key: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

impl KeyBinding {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            ctrl: false,
            alt: false,
            shift: false,
            meta: false,
        }
    }

    pub fn primary(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            ctrl: true,
            alt: false,
            shift: false,
            meta: true,
        }
    }

    pub fn matches(&self, event: &KeyBinding) -> bool {
        self.key.eq_ignore_ascii_case(&event.key)
            && self.alt == event.alt
            && self.shift == event.shift
            && (!self.ctrl || event.ctrl)
            && (!self.meta || event.meta || event.ctrl)
    }
}

pub type ActionPredicate = Rc<dyn Fn(&EditorState) -> bool>;
pub type ActionHandler = Rc<dyn Fn(EditorActionContext) -> Result<(), EditorError>>;

#[derive(Clone)]
pub struct EditorAction {
    pub id: String,
    pub label: String,
    pub group: Option<String>,
    pub icon: Option<String>,
    pub surfaces: Vec<ActionSurface>,
    pub shortcut: Option<KeyBinding>,
    pub priority: i32,
    pub is_enabled: ActionPredicate,
    pub is_active: ActionPredicate,
    pub run: ActionHandler,
}

impl EditorAction {
    pub fn enabled(&self, state: &EditorState) -> bool {
        (self.is_enabled)(state)
    }

    pub fn active(&self, state: &EditorState) -> bool {
        (self.is_active)(state)
    }
}

#[derive(Clone)]
pub struct EditorActionContext {
    pub catalog: EditorCatalog,
    pub state: EditorState,
}

impl EditorActionContext {
    pub fn dispatch_command(&mut self, id: &str, args: Value) -> Result<(), EditorError> {
        let transaction = self.catalog.commands().dispatch(id, &self.state, args)?;
        self.state.apply_transaction(transaction)
    }
}

#[derive(Clone, Default)]
pub struct ActionRegistry {
    actions: BTreeMap<String, EditorAction>,
}

impl ActionRegistry {
    pub fn register(&mut self, action: EditorAction) {
        self.actions.insert(action.id.clone(), action);
    }

    pub fn action(&self, id: &str) -> Option<EditorAction> {
        self.actions.get(id).cloned()
    }

    pub fn actions_for_surface(&self, surface: ActionSurface) -> Vec<EditorAction> {
        let mut actions = self
            .actions
            .values()
            .filter(|action| action.surfaces.contains(&surface))
            .cloned()
            .collect::<Vec<_>>();
        actions.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.label.cmp(&right.label))
        });
        actions
    }

    pub fn action_for_key(&self, key: &KeyBinding) -> Option<EditorAction> {
        self.actions
            .values()
            .filter(|action| {
                action
                    .shortcut
                    .as_ref()
                    .is_some_and(|binding| binding.matches(key))
            })
            .max_by_key(|action| action.priority)
            .cloned()
    }

    pub fn dispatch(
        &self,
        id: &str,
        catalog: EditorCatalog,
        state: EditorState,
    ) -> Result<(), EditorError> {
        let action = self
            .action(id)
            .ok_or_else(|| EditorError::UnknownAction(id.to_string()))?;
        if !action.enabled(&state) {
            return Ok(());
        }
        (action.run)(EditorActionContext { catalog, state })
    }

    pub fn dispatch_key(
        &self,
        key: &KeyBinding,
        catalog: EditorCatalog,
        state: EditorState,
    ) -> Result<bool, EditorError> {
        let Some(action) = self.action_for_key(key) else {
            return Ok(false);
        };
        if !action.enabled(&state) {
            return Ok(false);
        }
        (action.run)(EditorActionContext { catalog, state })?;
        Ok(true)
    }
}

pub fn register_standard_actions(registry: &mut ActionRegistry) {
    let always_enabled: ActionPredicate = Rc::new(|_| true);
    let never_active: ActionPredicate = Rc::new(|_| false);

    register_command_action(
        registry,
        "editor.undo",
        "Undo",
        Some("history"),
        vec![ActionSurface::MainToolbar, ActionSurface::CommandPalette],
        Some(KeyBinding::primary("z")),
        100,
        "editor.noop",
        Value::Null,
    );
    register_command_action(
        registry,
        "editor.redo",
        "Redo",
        Some("history"),
        vec![ActionSurface::MainToolbar, ActionSurface::CommandPalette],
        Some(KeyBinding {
            key: "z".to_string(),
            ctrl: true,
            alt: false,
            shift: true,
            meta: true,
        }),
        99,
        "editor.noop",
        Value::Null,
    );

    for (id, label, key, mark) in [
        ("editor.bold", "Bold", "b", "bold"),
        ("editor.italic", "Italic", "i", "italic"),
        ("editor.code", "Code", "e", "code"),
    ] {
        registry.register(EditorAction {
            id: id.to_string(),
            label: label.to_string(),
            group: Some("format".to_string()),
            icon: Some(label.to_ascii_lowercase()),
            surfaces: vec![
                ActionSurface::MainToolbar,
                ActionSurface::FloatingToolbar,
                ActionSurface::CommandPalette,
            ],
            shortcut: Some(KeyBinding::primary(key)),
            priority: 50,
            is_enabled: always_enabled.clone(),
            is_active: never_active.clone(),
            run: Rc::new(move |mut ctx| {
                ctx.dispatch_command("editor.toggle_mark", json!({ "mark": mark }))
            }),
        });
    }

    register_command_action(
        registry,
        "editor.paragraph",
        "Paragraph",
        Some("block"),
        vec![ActionSurface::SlashMenu, ActionSurface::CommandPalette],
        None,
        80,
        "editor.set_block_type",
        json!({ "component": "paragraph" }),
    );

    for level in 1..=3 {
        register_command_action(
            registry,
            format!("editor.heading_{level}"),
            format!("Heading {level}"),
            Some("block"),
            vec![ActionSurface::SlashMenu, ActionSurface::CommandPalette],
            None,
            79 - level,
            "editor.set_block_type",
            json!({ "component": "heading", "attrs": { "level": level } }),
        );
    }

    for (id, label) in [
        ("editor.link.insert", "Insert Link"),
        ("editor.link.edit", "Edit Link"),
        ("editor.link.remove", "Remove Link"),
        ("semantic.entity.insert_mention", "Insert Entity Mention"),
        ("semantic.entity.open", "Open Entity"),
        ("semantic.entity.copy_id", "Copy Entity Id"),
        (
            "semantic.entity.convert_selection_to_link",
            "Convert Selection To Entity Link",
        ),
    ] {
        registry.register(EditorAction {
            id: id.to_string(),
            label: label.to_string(),
            group: Some("semantic".to_string()),
            icon: None,
            surfaces: vec![ActionSurface::CommandPalette],
            shortcut: None,
            priority: 10,
            is_enabled: always_enabled.clone(),
            is_active: never_active.clone(),
            run: Rc::new(|mut ctx| ctx.dispatch_command("editor.noop", Value::Null)),
        });
    }
}

fn register_command_action(
    registry: &mut ActionRegistry,
    id: impl Into<String>,
    label: impl Into<String>,
    group: Option<&str>,
    surfaces: Vec<ActionSurface>,
    shortcut: Option<KeyBinding>,
    priority: i32,
    command: &'static str,
    args: Value,
) {
    registry.register(EditorAction {
        id: id.into(),
        label: label.into(),
        group: group.map(ToString::to_string),
        icon: None,
        surfaces,
        shortcut,
        priority,
        is_enabled: Rc::new(|_| true),
        is_active: Rc::new(|_| false),
        run: Rc::new(move |mut ctx| ctx.dispatch_command(command, args.clone())),
    });
}
