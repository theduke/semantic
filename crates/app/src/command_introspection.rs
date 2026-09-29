//! `semantic.command.*` commands: introspection of the registered commands.
//!
//! Types are encoded as facet-json strings, like `semantic.db.catalog`.

use std::future::Future;
use std::pin::Pin;

use semantic_data::schema::Type;
use semantic_data::value::{FromValue, IntoValue, SemanticType};
use semantic_rpc::RpcRegistry;
use semantic_rpc_core::{CommandDef, RpcCommand, RpcCommandSpec};

use crate::command::DocumentFormat;
use crate::{AppError, AppRequestContext};

pub(crate) fn register(
    registry: &mut RpcRegistry<AppRequestContext, AppError>,
) -> Result<(), AppError> {
    registry.register(List)?;
    registry.register(Get)?;
    Ok(())
}

struct List;
struct Get;

type CommandFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>;

#[derive(SemanticType, IntoValue, FromValue, Default)]
#[semantic(namespace = "semantic:command")]
struct ListPayload {
    /// Include the encoded input and output types.
    schema: Option<bool>,
}

#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(namespace = "semantic:command")]
struct ListOutput {
    /// The type encoding, with `schema`.
    format: Option<DocumentFormat>,
    commands: Vec<CommandEntry>,
}

/// A command's name, plus its encoded types with `schema`.
#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(namespace = "semantic:command")]
struct CommandEntry {
    name: String,
    input: Option<String>,
    output: Option<String>,
}

#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(namespace = "semantic:command")]
struct GetPayload {
    name: String,
}

#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(namespace = "semantic:command")]
struct GetOutput {
    format: DocumentFormat,
    name: String,
    input: String,
    output: String,
}

impl RpcCommandSpec for List {
    /// Null and void are accepted as an empty payload.
    type Payload = Option<ListPayload>;
    type Output = ListOutput;
    type Error = AppError;

    const NAME: &'static str = "semantic.command.list";
}

impl RpcCommand<AppRequestContext> for List {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: Option<ListPayload>,
    ) -> CommandFuture<'a, ListOutput> {
        Box::pin(async move {
            let schema = payload.unwrap_or_default().schema.unwrap_or(false);
            let commands = ctx
                .app
                .registry()
                .commands()
                .map(|command| {
                    let definition = command.definition();
                    let (input, output) = if schema {
                        (
                            Some(encode_type(&definition.input)?),
                            Some(encode_type(&definition.output)?),
                        )
                    } else {
                        (None, None)
                    };
                    Ok(CommandEntry {
                        name: definition.name.clone(),
                        input,
                        output,
                    })
                })
                .collect::<Result<Vec<_>, AppError>>()?;
            Ok(ListOutput {
                format: schema.then_some(DocumentFormat::FacetJson),
                commands,
            })
        })
    }
}

impl RpcCommandSpec for Get {
    type Payload = GetPayload;
    type Output = GetOutput;
    type Error = AppError;

    const NAME: &'static str = "semantic.command.get";
}

impl RpcCommand<AppRequestContext> for Get {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: GetPayload,
    ) -> CommandFuture<'a, GetOutput> {
        Box::pin(async move {
            let command = ctx
                .app
                .registry()
                .get(&payload.name)
                .ok_or(AppError::UnknownCommand(payload.name))?;
            let definition: &CommandDef = command.definition();
            Ok(GetOutput {
                format: DocumentFormat::FacetJson,
                name: definition.name.clone(),
                input: encode_type(&definition.input)?,
                output: encode_type(&definition.output)?,
            })
        })
    }
}

fn encode_type(ty: &Type) -> Result<String, AppError> {
    facet_json::to_string(ty).map_err(|err| AppError::InvalidRequest(err.to_string()))
}
