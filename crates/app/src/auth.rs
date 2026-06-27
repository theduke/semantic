#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrincipalId(String);

impl PrincipalId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PrincipalId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for PrincipalId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for PrincipalId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrincipalKind {
    User,
    Service,
    System,
}

#[derive(Clone, Debug)]
pub struct Principal {
    pub id: PrincipalId,
    pub kind: PrincipalKind,
}

impl Principal {
    pub fn system() -> Self {
        Self {
            id: PrincipalId::new("system"),
            kind: PrincipalKind::System,
        }
    }

    pub fn user(id: impl Into<String>) -> Self {
        Self {
            id: PrincipalId::new(id),
            kind: PrincipalKind::User,
        }
    }

    pub fn service(id: impl Into<String>) -> Self {
        Self {
            id: PrincipalId::new(id),
            kind: PrincipalKind::Service,
        }
    }
}
