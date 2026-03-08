macro_rules! local_id {
    ($name:ident) => {
        #[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub usize);

        impl From<usize> for $name {
            fn from(value: usize) -> Self {
                Self(value)
            }
        }

        impl From<$name> for usize {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

local_id!(LocalAttrId);
local_id!(LocalFieldId);
local_id!(LocalTypeDefId);
local_id!(LocalRecordTypeId);
local_id!(LocalClassId);
local_id!(LocalCollectionId);
local_id!(LocalIndexId);
local_id!(LocalRelationId);
