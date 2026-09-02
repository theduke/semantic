use semantic_data::value::{Object, Value};

use crate::CliError;
use crate::cmd::shared::{ApiClientArgs, FileInputArgs, OutputArgs};

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(flatten)]
    pub input: FileInputArgs,

    #[command(flatten)]
    pub client: ApiClientArgs,

    #[command(flatten)]
    pub output: OutputArgs,
}

pub async fn run(args: Args) -> std::result::Result<(), CliError> {
    let mut payload = normalize_payload(args.input.read_json()?)?;
    args.client.insert_scope(&mut payload);
    let response = args
        .client
        .rpc_client()
        .invoke_value("semantic.db.batch", Value::Object(payload))
        .await?;
    args.output.print(&response)
}

fn normalize_payload(value: Value) -> std::result::Result<Object, CliError> {
    match value {
        Value::Object(object) => Ok(object),
        Value::List(operations) => {
            let mut object = Object::new();
            object.insert("operations", Value::List(operations));
            Ok(object)
        }
        _ => Err(CliError::InvalidInput(
            "apply input must be a JSON object or an array of operations".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use semantic_data::value::Value;

    use super::normalize_payload;

    #[test]
    fn bare_operation_array_is_wrapped_for_current_batch_schema() {
        let payload = normalize_payload(Value::List(vec![Value::Object(
            [(
                "kind".to_string(),
                Value::String("delete_by_id".to_string()),
            )]
            .into_iter()
            .collect(),
        )]))
        .expect("normalize payload");
        assert!(
            matches!(payload.get("operations"), Some(Value::List(values)) if values.len() == 1)
        );
    }
}
