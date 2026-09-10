//! Streaming fetch and ordinary per-item import publication.
mod url;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
pub use semantic_data::import::*;
use semantic_jobs::{JobContext, JobError, JobHandler, JobKindDescriptor, JobKindId, JobProgress};
use semantic_plugin::PluginBinding;
use semantic_rpc::interface::{
    InvocationArgument, InvocationOutput, OwnedValueStream, StreamEvent,
};
use semantic_rpc_core::interface::InvocationError;
use sha2::{Digest, Sha256};
use std::{future::Future, pin::Pin, sync::Arc};
pub use url::GenericUrlPlugin;

pub enum ContentFrame {
    Item(ContentEvent),
    End(ContentSummary),
}
pub type ContentStream =
    Pin<Box<dyn Stream<Item = Result<ContentFrame, ImportError>> + Send + 'static>>;

/// Length-delimited identity excludes revision, job and content hash.
pub fn stable_entity_id(namespace: &str, source: &str, key: &str) -> semantic_data::Uuid {
    let mut hash = Sha256::new();
    hash.update(b"semantic.import.identity.v1\0");
    for part in [namespace, source, key] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    let digest = hash.finalize();
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes).into()
}

#[derive(Default)]
pub struct ContentValidator {
    file: Option<(Option<u64>, u64)>,
    summary: ContentSummary,
    ended: bool,
}
impl ContentValidator {
    pub fn accept(&mut self, frame: &ContentFrame) -> Result<(), ImportError> {
        let invalid = |message| ImportError::new("invalid_content", message);
        if self.ended {
            return Err(invalid("content after terminal"));
        }
        match frame {
            ContentFrame::Item(ContentEvent::Entity(e)) => {
                if self.file.is_some() || e.key.is_empty() || e.class.is_empty() {
                    return Err(invalid("invalid or interleaved entity"));
                }
                self.summary.items = self
                    .summary
                    .items
                    .checked_add(1)
                    .ok_or_else(|| invalid("item count overflow"))?;
            }
            ContentFrame::Item(ContentEvent::FileStart(f)) => {
                if self.file.is_some() || f.key.is_empty() || f.mime_type.is_empty() {
                    return Err(invalid("invalid or nested file"));
                }
                self.file = Some((f.expected_size, 0));
            }
            ContentFrame::Item(ContentEvent::FileBytes { bytes }) => {
                let Some((expected, size)) = self.file.as_mut() else {
                    return Err(invalid("bytes outside file"));
                };
                *size = size
                    .checked_add(bytes.len() as u64)
                    .ok_or_else(|| invalid("byte count overflow"))?;
                if expected.is_some_and(|n| *size > n) {
                    return Err(invalid("file exceeds expected length"));
                }
                self.summary.bytes = self
                    .summary
                    .bytes
                    .checked_add(bytes.len() as u64)
                    .ok_or_else(|| invalid("byte count overflow"))?;
            }
            ContentFrame::Item(ContentEvent::FileEnd) => {
                let Some((expected, size)) = self.file.take() else {
                    return Err(invalid("end outside file"));
                };
                if expected.is_some_and(|n| n != size) {
                    return Err(invalid("file length mismatch"));
                }
                self.summary.items = self
                    .summary
                    .items
                    .checked_add(1)
                    .ok_or_else(|| invalid("item count overflow"))?;
            }
            ContentFrame::End(summary) => {
                if self.file.is_some() {
                    return Err(invalid("incomplete file"));
                }
                if *summary != self.summary {
                    return Err(invalid("incorrect content summary"));
                }
                self.ended = true;
            }
        }
        Ok(())
    }
    pub fn finish(&self) -> Result<(), ImportError> {
        if self.ended {
            Ok(())
        } else {
            Err(ImportError::new(
                "incomplete_content",
                "missing successful terminal",
            ))
        }
    }
}

pub fn decode_stream(mut stream: OwnedValueStream) -> ContentStream {
    Box::pin(async_stream::try_stream! {
        let mut validator=ContentValidator::default();
        while let Some(frame)=stream.next().await {
            let frame=match frame.map_err(|e|ImportError::new(e.code,e.message))? {
                StreamEvent::Item(value)=>ContentFrame::Item(ContentEvent::from_value(value)?),
                StreamEvent::End(Some(value))=>ContentFrame::End(ContentSummary::from_value(value)?),
                StreamEvent::End(None)=>Err(ImportError::new("invalid_content","missing summary"))?,
            };
            validator.accept(&frame)?; yield frame;
        }
        validator.finish()?;
    })
}
pub fn encode_stream(mut stream: ContentStream) -> OwnedValueStream {
    OwnedValueStream::new(async_stream::try_stream! {
        let mut validator=ContentValidator::default();
        while let Some(frame)=stream.next().await {
            let frame=frame.map_err(invocation_error)?; validator.accept(&frame).map_err(invocation_error)?;
            yield match frame {ContentFrame::Item(event)=>StreamEvent::Item(event.to_value()),ContentFrame::End(summary)=>StreamEvent::End(Some(summary.to_value()))};
        }
        validator.finish().map_err(invocation_error)?;
    })
}
pub(crate) fn invocation_error(e: ImportError) -> InvocationError {
    let mut data = semantic_data::Object::new();
    data.insert("code", e.code.clone());
    data.insert("message", e.message.clone());
    InvocationError {
        code: e.code,
        message: e.message,
        data: Some(semantic_data::Value::Object(data)),
    }
}

#[async_trait::async_trait]
pub trait FileWrite: Send {
    async fn write_chunk(&mut self, bytes: Bytes) -> Result<(), ImportError>;
    /// Publication is admitted before this call and must finish despite cancellation.
    async fn finish(self: Box<Self>) -> Result<(), ImportError>;
    async fn abort(self: Box<Self>);
}
#[async_trait::async_trait]
pub trait ImportWriter: Send + Sync + 'static {
    async fn begin_file(
        &self,
        identity: &SourceIdentity,
        metadata: &FileMetadata,
    ) -> Result<Box<dyn FileWrite>, ImportError>;
    async fn write_entity(
        &self,
        identity: &SourceIdentity,
        entity: EntityProposal,
    ) -> Result<(), ImportError>;
}

pub enum ImportInput {
    Source(SourceRequest),
    Fetched {
        request: FetchedRequest,
        content: OwnedValueStream,
    },
}
pub struct ImportJobInput {
    pub binding: PluginBinding,
    pub identity: SourceIdentity,
    pub input: ImportInput,
    pub writer: Arc<dyn ImportWriter>,
}
pub struct ImportJobHandler {
    kind: JobKindDescriptor,
}
impl Default for ImportJobHandler {
    fn default() -> Self {
        Self {
            kind: JobKindDescriptor {
                id: JobKindId("semantic.import".into()),
                title: "Import".into(),
                description: Some("Publish source content as ordinary entities and files".into()),
            },
        }
    }
}
impl JobHandler for ImportJobHandler {
    type Input = ImportJobInput;
    type Output = ContentSummary;
    fn kind(&self) -> &JobKindDescriptor {
        &self.kind
    }
    fn run<'a>(
        &'a self,
        input: Self::Input,
        context: JobContext,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Output, JobError>> + Send + 'a>> {
        Box::pin(async move {
            run_import(input, context)
                .await
                .map_err(|e| JobError::new(e.code, e.message))
        })
    }
}
async fn run_import(
    input: ImportJobInput,
    context: JobContext,
) -> Result<ContentSummary, ImportError> {
    let check = || {
        context
            .check_cancelled()
            .map_err(|e| ImportError::new(e.code, e.message))
    };
    check()?;
    let (method, args) = match input.input {
        ImportInput::Source(r) => (
            "import_source",
            vec![InvocationArgument::Value(r.to_value())],
        ),
        ImportInput::Fetched { request, content } => (
            "import_fetched",
            vec![
                InvocationArgument::Value(request.to_value()),
                InvocationArgument::Stream(content),
            ],
        ),
    };
    let output = tokio::select! { _=context.cancellation().cancelled()=>return Err(ImportError::new("cancelled","import cancelled")), result=input.binding.invoke(method,args)=>result.map_err(|e|ImportError::new(e.code,e.message))? };
    let InvocationOutput::Stream(output) = output else {
        return Err(ImportError::new(
            "invalid_output",
            "expected content stream",
        ));
    };
    let mut stream = decode_stream(output);
    let mut file: Option<Box<dyn FileWrite>> = None;
    let mut summary = ContentSummary::default();
    let result:Result<ContentSummary,ImportError>=async {
        loop {
            let next=tokio::select! { _=context.cancellation().cancelled()=>return Err(ImportError::new("cancelled","import cancelled")), next=stream.next()=>next };
            let Some(frame)=next else{return Err(ImportError::new("incomplete_content","missing terminal"));};
            check()?;
            match frame? {
                ContentFrame::Item(ContentEvent::Entity(e))=>{input.writer.write_entity(&input.identity,e).await?;summary.items+=1;}
                ContentFrame::Item(ContentEvent::FileStart(metadata))=>{file=Some(input.writer.begin_file(&input.identity,&metadata).await?);}
                ContentFrame::Item(ContentEvent::FileBytes {bytes})=>{summary.bytes+=bytes.len() as u64;file.as_mut().expect("validated file frame").write_chunk(bytes).await?;}
                ContentFrame::Item(ContentEvent::FileEnd)=>{file.take().expect("validated file frame").finish().await?;summary.items+=1;}
                ContentFrame::End(_)=>return Ok(summary),
            }
            context.report_progress(JobProgress {completed:summary.items,total:None,unit:Some("items".into()),phase:Some("import".into())}).map_err(|e|ImportError::new("invalid_progress",e.to_string()))?;
        }
    }.await;
    if let Some(file) = file {
        file.abort().await;
    }
    result
}

/// Fetch carries upstream groups outside peer-provided content values.
pub struct FetchedContent {
    pub identity: SourceIdentity,
    pub stream: OwnedValueStream,
    pub groups: Vec<semantic_jobs::JobGroup>,
}
pub async fn fetch(
    binding: &PluginBinding,
    identity: SourceIdentity,
    request: SourceRequest,
) -> Result<FetchedContent, ImportError> {
    match binding
        .invoke("fetch", vec![InvocationArgument::Value(request.to_value())])
        .await
        .map_err(|e| ImportError::new(e.code, e.message))?
    {
        InvocationOutput::Stream(stream) => Ok(FetchedContent {
            identity,
            stream: encode_stream(decode_stream(stream)),
            groups: vec![binding.group()],
        }),
        _ => Err(ImportError::new("invalid_output", "expected fetch stream")),
    }
}

pub struct SourceCandidate {
    pub source: PluginBinding,
    pub operation: PluginBinding,
    pub descriptor: SourceDescriptor,
    pub priority: i32,
    pub probe: ProbeResult,
}
/// Probe ordered immutable bindings; callers retain the selected operation binding.
pub async fn discover(
    mut sources: Vec<(PluginBinding, PluginBinding, SourceDescriptor)>,
    request: &SourceRequest,
    operation: Operation,
) -> Vec<SourceCandidate> {
    sources.retain(|(_, _, d)| d.operations.contains(&operation));
    sources.sort_by(|(a, operation_a, ad), (b, operation_b, bd)| {
        b.priority()
            .unwrap_or(bd.default_priority)
            .cmp(&a.priority().unwrap_or(ad.default_priority))
            .then_with(|| a.plugin_id().cmp(b.plugin_id()))
            .then_with(|| {
                operation_a
                    .descriptor()
                    .export
                    .cmp(&operation_b.descriptor().export)
            })
    });
    let mut out = Vec::new();
    for (source, binding, descriptor) in sources {
        if !descriptor.url_schemes.is_empty()
            && !::url::Url::parse(&request.url)
                .ok()
                .is_some_and(|url| descriptor.url_schemes.iter().any(|s| s == url.scheme()))
        {
            continue;
        }
        let probe = match source
            .invoke(
                "probe",
                vec![
                    InvocationArgument::Value(request.to_value()),
                    InvocationArgument::Value(operation.as_str().to_owned().into()),
                ],
            )
            .await
        {
            Ok(InvocationOutput::Values(mut values)) if values.len() == 1 => match values.remove(0)
            {
                semantic_data::Value::Object(mut o) => match take_string(&mut o, "status")
                    .as_deref()
                {
                    Ok("supported") => ProbeResult::Supported,
                    Ok("unsupported") => {
                        ProbeResult::Unsupported(take_string(&mut o, "reason").unwrap_or_default())
                    }
                    _ => ProbeResult::Unavailable(
                        take_string(&mut o, "reason").unwrap_or_else(|_| "invalid probe".into()),
                    ),
                },
                _ => ProbeResult::Unavailable("invalid probe".into()),
            },
            Err(e) => ProbeResult::Unavailable(e.to_string()),
            _ => ProbeResult::Unavailable("invalid probe".into()),
        };
        out.push(SourceCandidate {
            priority: source.priority().unwrap_or(descriptor.default_priority),
            source,
            operation: binding,
            descriptor,
            probe,
        });
    }
    out
}
pub fn select(
    candidates: &[SourceCandidate],
    explicit: Option<(&str, &str)>,
) -> Result<PluginBinding, ImportError> {
    candidates
        .iter()
        .find(|c| {
            matches!(c.probe, ProbeResult::Supported)
                && explicit.is_none_or(|(plugin, export)| {
                    c.operation.plugin_id() == plugin && c.operation.descriptor().export == export
                })
        })
        .map(|c| c.operation.clone())
        .ok_or_else(|| ImportError::new("unsupported_source", "no supported matching source"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn start(size: Option<u64>) -> ContentFrame {
        ContentFrame::Item(ContentEvent::FileStart(FileMetadata {
            key: "file".into(),
            filename: None,
            mime_type: "text/plain".into(),
            expected_size: size,
            attributes: semantic_data::Object::new(),
        }))
    }
    #[test]
    fn identities_use_unambiguous_components() {
        assert_ne!(
            stable_entity_id("ab", "c", "d"),
            stable_entity_id("a", "bc", "d")
        );
        assert_ne!(
            stable_entity_id("a", "b", "c"),
            stable_entity_id("a", "b", "d")
        );
        assert_eq!(
            stable_entity_id("a", "b", "c"),
            stable_entity_id("a", "b", "c")
        );
    }
    #[test]
    fn file_framing_length_and_terminal() {
        let mut v = ContentValidator::default();
        assert!(
            v.accept(&ContentFrame::Item(ContentEvent::FileEnd))
                .is_err()
        );
        v.accept(&start(Some(0))).unwrap();
        assert!(v.accept(&start(None)).is_err());
        v.accept(&ContentFrame::Item(ContentEvent::FileEnd))
            .unwrap();
        v.accept(&ContentFrame::End(ContentSummary { items: 1, bytes: 0 }))
            .unwrap();
        v.finish().unwrap();
        assert!(v.accept(&start(None)).is_err());
        let mut v = ContentValidator::default();
        v.accept(&start(Some(2))).unwrap();
        assert!(
            v.accept(&ContentFrame::Item(ContentEvent::FileEnd))
                .is_err()
        );
        assert!(v.finish().is_err());
    }
    #[test]
    fn incomplete_file_and_forged_summary_rejected() {
        let mut v = ContentValidator::default();
        v.accept(&start(None)).unwrap();
        assert!(
            v.accept(&ContentFrame::End(ContentSummary::default()))
                .is_err()
        );
        assert!(
            ContentValidator::default()
                .accept(&ContentFrame::End(ContentSummary { items: 1, bytes: 0 }))
                .is_err()
        );
    }
    #[tokio::test]
    async fn missing_stream_end_fails() {
        let stream = OwnedValueStream::new(futures_util::stream::empty());
        assert!(decode_stream(stream).next().await.unwrap().is_err());
    }
}
