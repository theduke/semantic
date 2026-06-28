use crate::catalog::NameSet;

pub const IMPLICIT_ROOT_PACKAGE: &str = "local";
pub const SEMANTIC_PACKAGE: &str = "semantic";

pub fn nameset_for_identifier(identifier: &str, module_hint: Option<&str>) -> NameSet {
    if is_special_builtin_field(identifier) {
        return NameSet {
            qualified_name: identifier.to_string(),
            plain_name: identifier.to_string(),
            underscore_name: identifier.to_string(),
        };
    }

    let (package, module, plain) = split_identifier(identifier, module_hint);
    let qualified_name = if let Some(module_name) = module.as_deref() {
        format!("{package}:{module_name}:{plain}")
    } else {
        format!("{package}:{plain}")
    };
    let underscore_name = if let Some(module_name) = module.as_deref() {
        format!("{package}_{module_name}_{plain}")
    } else {
        format!("{package}_{plain}")
    };
    NameSet {
        qualified_name,
        plain_name: plain.to_string(),
        underscore_name,
    }
}

pub fn nameset_for_qualified(qualified_name: &str) -> NameSet {
    if is_special_builtin_field(qualified_name) {
        return NameSet {
            qualified_name: qualified_name.to_string(),
            plain_name: qualified_name.to_string(),
            underscore_name: qualified_name.to_string(),
        };
    }

    let mut parts = qualified_name.split(':').collect::<Vec<_>>();
    if parts.len() < 2 {
        return nameset_for_identifier(qualified_name, None);
    }

    let plain = parts.pop().unwrap_or(qualified_name);
    let package = parts.first().copied().unwrap_or(IMPLICIT_ROOT_PACKAGE);
    let module = if parts.len() > 1 {
        Some(parts[1..].join(":"))
    } else {
        None
    };
    let underscore_name = if let Some(module) = module.as_deref() {
        format!("{package}_{module}_{plain}")
    } else {
        format!("{package}_{plain}")
    };
    NameSet {
        qualified_name: qualified_name.to_string(),
        plain_name: plain.to_string(),
        underscore_name,
    }
}

pub fn is_special_builtin_field(name: &str) -> bool {
    name == semantic_data::builtin::ATTR_ID || name == semantic_data::builtin::ATTR_TYPE
}

fn split_identifier<'a>(
    identifier: &'a str,
    module_hint: Option<&'a str>,
) -> (&'a str, Option<String>, &'a str) {
    if identifier.contains(':') {
        let mut parts = identifier.split(':').collect::<Vec<_>>();
        let plain = parts.pop().unwrap_or(identifier);
        let package = parts.first().copied().unwrap_or(IMPLICIT_ROOT_PACKAGE);
        let module = if parts.len() > 1 {
            Some(parts[1..].join(":"))
        } else {
            None
        };
        return (package, module, plain);
    }

    if identifier.contains('.') {
        let parts = identifier.split('.').collect::<Vec<_>>();
        if parts.len() == 2 {
            return (parts[0], None, parts[1]);
        }
        if parts.len() >= 3 {
            return (
                parts[0],
                Some(parts[1..parts.len() - 1].join(":")),
                parts[parts.len() - 1],
            );
        }
    }

    if let Some((package, plain)) = identifier.split_once('_')
        && package == SEMANTIC_PACKAGE
        && !plain.is_empty()
    {
        return (package, None, plain);
    }

    let package = IMPLICIT_ROOT_PACKAGE;
    let module = module_hint.map(ToString::to_string);
    (package, module, identifier)
}
