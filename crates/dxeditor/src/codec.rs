use std::{collections::BTreeMap, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    EditorError,
    document::{DOCUMENT_SCHEMA_V1, EditorDocument},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditorPayload {
    pub format: String,
    pub value: Value,
}

impl EditorPayload {
    pub fn new(format: impl Into<String>, value: Value) -> Self {
        Self {
            format: format.into(),
            value,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DecodeContext {
    pub fallback_unknown_components: bool,
}

#[derive(Clone, Debug)]
pub struct EncodeContext {
    pub fallback_unknown_components: bool,
}

impl Default for EncodeContext {
    fn default() -> Self {
        Self {
            fallback_unknown_components: true,
        }
    }
}

pub trait EditorCodec: Send + Sync {
    fn format(&self) -> &str;

    fn decode(
        &self,
        payload: &EditorPayload,
        ctx: DecodeContext,
    ) -> Result<EditorDocument, EditorError>;

    fn encode(
        &self,
        document: &EditorDocument,
        ctx: EncodeContext,
    ) -> Result<EditorPayload, EditorError>;
}

#[derive(Clone, Default)]
pub struct CodecRegistry {
    codecs: BTreeMap<String, Arc<dyn EditorCodec>>,
}

impl CodecRegistry {
    pub fn register(&mut self, codec: Arc<dyn EditorCodec>) {
        self.codecs.insert(codec.format().to_string(), codec);
    }

    pub fn codec(&self, format: &str) -> Option<Arc<dyn EditorCodec>> {
        self.codecs.get(format).cloned()
    }

    pub fn decode(&self, payload: &EditorPayload) -> Result<EditorDocument, EditorError> {
        self.decode_with(payload, DecodeContext::default())
    }

    pub fn decode_with(
        &self,
        payload: &EditorPayload,
        ctx: DecodeContext,
    ) -> Result<EditorDocument, EditorError> {
        let codec = self
            .codec(&payload.format)
            .ok_or_else(|| EditorError::UnknownFormat(payload.format.clone()))?;
        codec.decode(payload, ctx)
    }

    pub fn encode(
        &self,
        document: &EditorDocument,
        format: &str,
    ) -> Result<EditorPayload, EditorError> {
        self.encode_with(document, format, EncodeContext::default())
    }

    pub fn encode_with(
        &self,
        document: &EditorDocument,
        format: &str,
        ctx: EncodeContext,
    ) -> Result<EditorPayload, EditorError> {
        let codec = self
            .codec(format)
            .ok_or_else(|| EditorError::UnknownFormat(format.to_string()))?;
        codec.encode(document, ctx)
    }

    pub fn formats(&self) -> impl Iterator<Item = &str> {
        self.codecs.keys().map(String::as_str)
    }
}

pub fn register_standard_codecs(registry: &mut CodecRegistry) {
    registry.register(Arc::new(DocumentCodec));
    registry.register(Arc::new(PlainTextCodec));
    #[cfg(feature = "markdown")]
    registry.register(Arc::new(crate::markdown::MarkdownCodec));
}

pub struct DocumentCodec;

impl EditorCodec for DocumentCodec {
    fn format(&self) -> &str {
        DOCUMENT_SCHEMA_V1
    }

    fn decode(
        &self,
        payload: &EditorPayload,
        _ctx: DecodeContext,
    ) -> Result<EditorDocument, EditorError> {
        serde_json::from_value(payload.value.clone()).map_err(|err| EditorError::InvalidPayload {
            format: payload.format.clone(),
            message: err.to_string(),
        })
    }

    fn encode(
        &self,
        document: &EditorDocument,
        _ctx: EncodeContext,
    ) -> Result<EditorPayload, EditorError> {
        Ok(EditorPayload::new(
            DOCUMENT_SCHEMA_V1,
            serde_json::to_value(document).map_err(|err| EditorError::InvalidPayload {
                format: DOCUMENT_SCHEMA_V1.to_string(),
                message: err.to_string(),
            })?,
        ))
    }
}

pub struct PlainTextCodec;

impl EditorCodec for PlainTextCodec {
    fn format(&self) -> &str {
        "plain_text"
    }

    fn decode(
        &self,
        payload: &EditorPayload,
        _ctx: DecodeContext,
    ) -> Result<EditorDocument, EditorError> {
        let Some(text) = payload.value.as_str() else {
            return Err(EditorError::InvalidPayload {
                format: payload.format.clone(),
                message: "expected string".to_string(),
            });
        };
        Ok(EditorDocument::plain_text(text))
    }

    fn encode(
        &self,
        document: &EditorDocument,
        _ctx: EncodeContext,
    ) -> Result<EditorPayload, EditorError> {
        Ok(EditorPayload::new(
            "plain_text",
            Value::String(document.text_content()),
        ))
    }
}
