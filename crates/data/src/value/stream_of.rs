//! Schema-only marker for stream types.

use std::fmt;
use std::marker::PhantomData;

use super::SemanticType;
use crate::schema::{StreamType, Type, TypeKind};

/// Describes a stream of `T` elements, optionally terminated by an `End` value.
///
/// This is a schema-only marker: it produces a [`TypeKind::Stream`] through
/// [`SemanticType`] but carries no runtime data and cannot be constructed.
/// It deliberately implements neither [`IntoValue`](super::IntoValue) nor
/// [`FromValue`](super::FromValue), so streams cannot be embedded in derived
/// value payloads; they only appear at the top level of signatures.
///
/// `End` defaults to `()`, meaning the stream has no end value. Any `End`
/// whose type is [`TypeKind::Never`] is treated the same way.
///
/// ```compile_fail
/// #[derive(semantic_data::IntoValue)]
/// struct Payload {
///     items: semantic_data::value::StreamOf<String>,
/// }
/// ```
pub struct StreamOf<T, End = ()> {
    _marker: PhantomData<fn() -> (T, End)>,
}

impl<T, End> fmt::Debug for StreamOf<T, End> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StreamOf")
    }
}

impl<T: SemanticType, End: SemanticType> SemanticType for StreamOf<T, End> {
    fn semantic_type() -> Type {
        let end = End::semantic_type();
        let end = (!matches!(end.kind, TypeKind::Never(_))).then(|| Box::new(end));
        Type::new(TypeKind::Stream(StreamType {
            element: Box::new(T::semantic_type()),
            end,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream_type<S: SemanticType>() -> StreamType {
        match S::semantic_type().kind {
            TypeKind::Stream(stream) => stream,
            other => panic!("expected stream type, got {other:?}"),
        }
    }

    #[test]
    fn stream_without_end_has_no_end_type() {
        let stream = stream_type::<StreamOf<String>>();
        assert_eq!(*stream.element, String::semantic_type());
        assert_eq!(stream.end, None);
    }

    #[test]
    fn stream_with_end_declares_end_type() {
        let stream = stream_type::<StreamOf<bool, Vec<u64>>>();
        assert_eq!(*stream.element, bool::semantic_type());
        assert_eq!(stream.end, Some(Box::new(Vec::<u64>::semantic_type())));
    }

    #[test]
    fn stream_type_round_trips() {
        let expected = Type::new(TypeKind::Stream(StreamType {
            element: Box::new(Option::<String>::semantic_type()),
            end: Some(Box::new(u32::semantic_type())),
        }));
        let ty = StreamOf::<Option<String>, u32>::semantic_type();
        assert_eq!(ty, expected);

        let encoded = facet_json::to_string(&ty).unwrap();
        assert_eq!(facet_json::from_str::<Type>(&encoded).unwrap(), expected);
    }
}
