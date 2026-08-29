mod shell;

pub use shell::{AppShell, PlayerShell};

pub(crate) fn value_string(value: &semantic_data::value::Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("{value:?}"))
}
