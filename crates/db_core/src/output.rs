use semantic_data::{query::FieldFormat, value::Object};

use crate::catalog::Catalog;

/// Format canonical output field names according to a query's requested format.
pub fn format_output_rows(
    catalog: &Catalog,
    rows: Vec<Object>,
    format: FieldFormat,
) -> Vec<Object> {
    rows.into_iter()
        .map(|row| format_output_object(catalog, row, format))
        .collect()
}

/// Format one canonical object without changing its values.
pub fn format_output_object(catalog: &Catalog, object: Object, format: FieldFormat) -> Object {
    if format == FieldFormat::Qualified {
        return object;
    }

    let mut out = Object::new();
    for (key, value) in object {
        let next_key = if let Some(attribute) = catalog.attribute_by_id(&key) {
            match format {
                FieldFormat::Qualified => attribute.names.qualified_name.clone(),
                FieldFormat::Underscore => attribute.names.underscore_name.clone(),
                FieldFormat::Plain => attribute.names.plain_name.clone(),
            }
        } else {
            key
        };
        out.insert(next_key, value);
    }
    out
}

#[cfg(test)]
mod tests {
    use semantic_data::query::FieldFormat;
    use semantic_data::value::{Object, Value};

    use super::*;

    #[test]
    fn unknown_fields_are_preserved() {
        let object = Object::from_iter([("expression".into(), Value::I32(2))]);
        assert_eq!(
            format_output_object(&Catalog::new(), object.clone(), FieldFormat::Plain),
            object
        );
    }
}
