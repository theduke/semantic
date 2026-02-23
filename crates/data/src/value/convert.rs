use super::{Value, ValueRef};

pub enum ConvertError {}

pub struct ConvertCtx {}

pub trait ValueConvert {
    fn as_value_ref<'a>(&'a self, ctx: &ConvertCtx) -> ValueRef<'a>;

    fn into_value(self, ctx: &ConvertCtx) -> Value
    where
        Self: Sized,
    {
        self.as_value_ref(ctx).into_owned()
    }

    fn from_value(value: Value, ctx: &ConvertCtx) -> Result<Self, ConvertError>
    where
        Self: Sized;

    fn from_value_ref(value_ref: ValueRef, ctx: &ConvertCtx) -> Result<Self, ConvertError>
    where
        Self: Sized,
    {
        Self::from_value(value_ref.into_owned(), ctx)
    }
}
