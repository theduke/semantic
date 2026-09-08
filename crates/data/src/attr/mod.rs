use crate::schema::AttributeType;

pub const ATTR_TITLE: &str = "semantic:title";

pub trait AttrDescriptor {
    fn attr_schema(&self) -> AttributeType;
}

pub trait AttrDescriptorConst: AttrDescriptor {
    const ID: &'static str;
    const PLAIN_NAME: &'static str;
}

//
// pub struct AttrFieldFormat;
