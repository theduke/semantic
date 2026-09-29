//! `semantic.command.*` commands: introspection of the registered commands.
//!
//! Types are encoded as facet-json strings, like `semantic.db.catalog`.

use std::future::Future;
use std::pin::Pin;

use semantic_data::schema::{
    Field, ListType, Meta, OptionalType, RecordType, StringType, Type, TypeKind,
};
use semantic_data::value::{Object, Value};
use semantic_rpc::RpcRegistry;
use semantic_rpc_core::{CommandDef, RpcCommand, RpcCommandSpec};

use crate::command::{expect_object, optional_bool, required_string};
use crate::{AppError, AppRequestContext};

const TYPE_FORMAT: &str = "facet-json";

pub(crate) fn register(
    registry: &mut RpcRegistry<AppRequestContext, AppError>,
) -> Result<(), AppError> {
    registry.register(List)?;
    registry.register(Get)?;
    Ok(())
}

struct List;
struct Get;

impl RpcCommandSpec for List {
    type Payload = Value;
    type Output = Value;
    type Error = AppError;

    const NAME: &'static str = "semantic.command.list";

    fn definition(&self) -> CommandDef {
        let entry = record(vec![
            ("name", string(), true),
            ("input", string(), false),
            ("output", string(), false),
        ]);
        CommandDef::new(
            Self::NAME,
            record(vec![("schema", optional(Type::new_bool()), false)]),
            record(vec![
                ("format", string(), false),
                ("commands", list(entry), true),
            ]),
        )
    }
}

impl RpcCommand<AppRequestContext> for List {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, AppError>> + Send + 'a>> {
        Box::pin(async move {
            let schema = match payload {
                Value::Void | Value::Null => false,
                payload => optional_bool(&expect_object(payload)?, "schema")?.unwrap_or(false),
            };
            let commands = ctx
                .app
                .registry()
                .commands()
                .map(|command| command_value(command.definition(), schema).map(Value::Object))
                .collect::<Result<Vec<_>, _>>()?;
            let mut out = Object::new();
            if schema {
                out.insert("format", TYPE_FORMAT.to_string());
            }
            out.insert("commands", Value::List(commands));
            Ok(Value::Object(out))
        })
    }
}

impl RpcCommandSpec for Get {
    type Payload = Value;
    type Output = Value;
    type Error = AppError;

    const NAME: &'static str = "semantic.command.get";

    fn definition(&self) -> CommandDef {
        CommandDef::new(
            Self::NAME,
            record(vec![("name", string(), true)]),
            record(vec![
                ("format", string(), true),
                ("name", string(), true),
                ("input", string(), true),
                ("output", string(), true),
            ]),
        )
    }
}

impl RpcCommand<AppRequestContext> for Get {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, AppError>> + Send + 'a>> {
        Box::pin(async move {
            let name = required_string(&expect_object(payload)?, "name")?;
            let command = ctx
                .app
                .registry()
                .get(&name)
                .ok_or(AppError::UnknownCommand(name))?;
            let mut out = command_value(command.definition(), true)?;
            out.insert("format", TYPE_FORMAT.to_string());
            Ok(Value::Object(out))
        })
    }
}

/// The command's name, plus its facet-json encoded types if `schema` is set.
fn command_value(definition: &CommandDef, schema: bool) -> Result<Object, AppError> {
    let mut out = Object::new();
    out.insert("name", definition.name.clone());
    if schema {
        out.insert("input", encode_type(&definition.input)?);
        out.insert("output", encode_type(&definition.output)?);
    }
    Ok(out)
}

fn encode_type(ty: &Type) -> Result<String, AppError> {
    facet_json::to_string(ty).map_err(|err| AppError::InvalidRequest(err.to_string()))
}

fn string() -> Type {
    Type::new(TypeKind::String(StringType {
        format: None,
        normalization: None,
    }))
}

fn optional(ty: Type) -> Type {
    Type::new(TypeKind::Optional(OptionalType {
        inner: Box::new(ty),
    }))
}

fn list(ty: Type) -> Type {
    Type::new(TypeKind::List(ListType {
        items: Box::new(ty),
    }))
}

fn record(fields: Vec<(&str, Type, bool)>) -> Type {
    Type::new(TypeKind::Record(RecordType {
        fields: fields
            .into_iter()
            .map(|(name, ty, required)| {
                (
                    name.into(),
                    Field {
                        ty,
                        required,
                        readonly: false,
                        writeonly: false,
                        default: None,
                        meta: Meta::default(),
                    },
                )
            })
            .collect(),
        open: false,
        additional: None,
        required_order: None,
    }))
}
