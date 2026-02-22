#[derive(facet::Facet, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[facet(transparent)]
pub struct Uuid(uuid::Uuid);

impl Uuid {
    pub const NIL: Self = Self(::uuid::Uuid::nil());
}

impl From<::uuid::Uuid> for Uuid {
    fn from(uuid: ::uuid::Uuid) -> Self {
        Self(uuid)
    }
}

impl From<Uuid> for ::uuid::Uuid {
    fn from(uuid: Uuid) -> Self {
        uuid.0
    }
}
