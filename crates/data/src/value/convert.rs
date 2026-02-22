use super::{Value, ValueRef};

pub enum ConvertError {}

pub struct ConvertCtx {}

pub trait ValueConvert {
    fn into_value(self, ctx: &ConvertCtx) -> Value;

    fn as_value_ref<'a>(&'a self, ctx: &ConvertCtx) -> ValueRef<'a>;

    fn from_value(value: Value, ctx: &ConvertCtx) -> Result<Self, ConvertError>
    where
        Self: Sized;

    fn from_value_ref(value_ref: ValueRef, ctx: &ConvertCtx) -> Result<Self, ConvertError>
    where
        Self: Sized;
}
