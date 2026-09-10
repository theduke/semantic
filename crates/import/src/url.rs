use super::*;
use semantic_data::{Object, Value};
use semantic_plugin::{Plugin, PluginError, PluginInstanceContext, PluginManifest};
use semantic_rpc::interface::{InterfaceImplementation, InvocationContext, ValidatedInvocation};
use semantic_rpc_core::interface::ImplementationDescriptor;

pub struct GenericUrlPlugin {
    manifest: PluginManifest,
    client: reqwest::Client,
}
impl GenericUrlPlugin {
    pub fn new(exports: Vec<ImplementationDescriptor>) -> Self {
        Self {
            manifest: PluginManifest {
                id: "semantic.generic-url".into(),
                revision: "1".into(),
                title: "Generic URL file".into(),
                exports,
                source_bindings: Default::default(),
                configuration_schema: Some(semantic_data::plugin::PluginConfigurationSchema {
                    ty: semantic_data::schema::Type::new(semantic_data::schema::TypeKind::Null(
                        semantic_data::schema::NullType,
                    )),
                    definitions: Default::default(),
                }),
            },
            client: reqwest::Client::new(),
        }
    }
    pub fn source_descriptor() -> SourceDescriptor {
        SourceDescriptor {
            namespace: "semantic.url-file".into(),
            title: "Generic URL file".into(),
            operations: vec![
                Operation::Fetch,
                Operation::ImportSource,
                Operation::ImportFetched,
            ],
            input_kinds: vec!["file".into()],
            url_schemes: vec!["http".into(), "https".into()],
            default_priority: -100,
        }
    }
    pub fn identity(request: &SourceRequest) -> Result<SourceIdentity, ImportError> {
        Ok(SourceIdentity {
            namespace: "semantic.url-file".into(),
            source: source_url(&request.url)?.to_string(),
        })
    }
}
impl Plugin for GenericUrlPlugin {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
    fn create<'a>(
        &'a self,
        _context: PluginInstanceContext,
    ) -> Pin<
        Box<dyn Future<Output = Result<Arc<dyn InterfaceImplementation>, PluginError>> + Send + 'a>,
    > {
        Box::pin(async move {
            Ok(Arc::new(UrlInstance {
                exports: self.manifest.exports.clone(),
                client: self.client.clone(),
            }) as Arc<dyn InterfaceImplementation>)
        })
    }
}
struct UrlInstance {
    exports: Vec<ImplementationDescriptor>,
    client: reqwest::Client,
}
impl InterfaceImplementation for UrlInstance {
    fn descriptors(&self) -> &[ImplementationDescriptor] {
        &self.exports
    }
    fn invoke<'a>(
        &'a self,
        call: ValidatedInvocation,
        context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<InvocationOutput, InvocationError>> + Send + 'a>> {
        Box::pin(async move {
            let export = self
                .exports
                .iter()
                .find(|export| export.export == call.export)
                .ok_or_else(|| {
                    InvocationError::new("unknown_export", "unknown URL plugin export")
                })?;
            let arity = match (export.interface.name.as_str(), call.method.as_str()) {
                ("Source", "describe") => 0,
                ("Source", "probe") => 2,
                ("Fetcher", "fetch") | ("Importer", "import_source") => 1,
                ("Importer", "import_fetched") => 2,
                _ => {
                    return Err(InvocationError::new(
                        "unknown_method",
                        "method does not belong to export",
                    ));
                }
            };
            if call.arguments.len() != arity {
                return Err(InvocationError::new(
                    "invalid_input",
                    "wrong argument count",
                ));
            }
            let mut args = call.arguments.into_iter();
            let request = |arg: Option<InvocationArgument>| match arg {
                Some(InvocationArgument::Value(value)) => Ok(value),
                _ => Err(InvocationError::new("invalid_input", "expected request")),
            };
            match call.method.as_str() {
                "describe" => Ok(InvocationOutput::Values(vec![
                    GenericUrlPlugin::source_descriptor().to_value(),
                ])),
                "probe" => {
                    let r = SourceRequest::from_value(request(args.next())?)
                        .map_err(invocation_error)?;
                    let Value::String(operation) = request(args.next())? else {
                        return Err(InvocationError::new("invalid_input", "expected operation"));
                    };
                    Operation::parse(&operation).map_err(invocation_error)?;
                    let valid = source_url(&r.url).is_ok();
                    let mut o = Object::new();
                    o.insert(
                        "status",
                        String::from(if valid { "supported" } else { "unsupported" }),
                    );
                    o.insert(
                        "reason",
                        if valid {
                            Value::Null
                        } else {
                            Value::String("expected HTTP or HTTPS URL".into())
                        },
                    );
                    Ok(InvocationOutput::Values(vec![Value::Object(o)]))
                }
                "fetch" | "import_source" => {
                    let r = SourceRequest::from_value(request(args.next())?)
                        .map_err(invocation_error)?;
                    let stream = read_url(self.client.clone(), r, context);
                    Ok(InvocationOutput::Stream(encode_stream(stream)))
                }
                "import_fetched" => {
                    let r = FetchedRequest::from_value(request(args.next())?)
                        .map_err(invocation_error)?;
                    if r.representation != "file" {
                        return Err(InvocationError::new(
                            "unsupported_input",
                            "expected file representation",
                        ));
                    }
                    let Some(InvocationArgument::Stream(stream)) = args.next() else {
                        return Err(InvocationError::new(
                            "invalid_input",
                            "expected content stream",
                        ));
                    };
                    let mut stream = decode_stream(stream);
                    let checked = Box::pin(async_stream::try_stream! {
                        loop {
                            let frame = tokio::select! {
                                biased;
                                _ = context.cancellation.cancelled() => Err(ImportError::new("cancelled", "plugin changed")),
                                frame = stream.next() => Ok(frame),
                            }?;
                            let Some(frame) = frame else { break };
                            let mut frame = frame?;
                            match &mut frame {
                                ContentFrame::Item(ContentEvent::Entity(_)) => Err(ImportError::new("unsupported_input", "generic importer accepts files only"))?,
                                ContentFrame::Item(ContentEvent::FileStart(metadata)) => {
                                    metadata.mime_type = accepted_mime(Some(&metadata.mime_type), metadata.filename.as_deref())?;
                                }
                                _ => {}
                            }
                            yield frame;
                        }
                    });
                    Ok(InvocationOutput::Stream(encode_stream(checked)))
                }
                _ => Err(InvocationError::new(
                    "unknown_method",
                    "unknown generic URL method",
                )),
            }
        })
    }
}
fn source_url(value: &str) -> Result<::url::Url, ImportError> {
    let url =
        ::url::Url::parse(value).map_err(|e| ImportError::new("invalid_source", e.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ImportError::new(
            "unsupported_source",
            "expected HTTP or HTTPS URL",
        ));
    }
    Ok(url)
}
fn read_url(
    client: reqwest::Client,
    request: SourceRequest,
    context: InvocationContext,
) -> ContentStream {
    Box::pin(async_stream::try_stream! {
        let url=source_url(&request.url)?;
        let response=tokio::select! {_=context.cancellation.cancelled()=>Err(ImportError::new("cancelled","fetch cancelled")),response=client.get(url).send()=>response.map_err(|e|ImportError::new("fetch_failed",e.to_string()))}?;
        source_url(response.url().as_str())?;
        if !response.status().is_success() {Err(ImportError::new("http_status",format!("HTTP {}",response.status())))?;}
        let path_name=response.url().path_segments().and_then(|segments|segments.filter(|v|!v.is_empty()).next_back()).map(str::to_owned);
        let filename=response.headers().get(reqwest::header::CONTENT_DISPOSITION).and_then(|v|v.to_str().ok()).and_then(disposition_filename).or_else(||path_name.clone()).unwrap_or_else(||"file".into());
        let raw_mime=response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v|v.to_str().ok());
        let mime=accepted_mime(raw_mime,Some(&filename)).or_else(|error| {
            let generic=raw_mime.is_none_or(|mime|matches!(mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase().as_str(),""|"application/octet-stream"));
            if generic {accepted_mime(raw_mime,path_name.as_deref())} else {Err(error)}
        })?;
        // Reqwest removes Content-Length when decoding an encoded response.
        let expected_size=if response.headers().contains_key(reqwest::header::CONTENT_ENCODING) {None} else {response.content_length()};
        yield ContentFrame::Item(ContentEvent::FileStart(FileMetadata {key:"file".into(),filename:Some(filename),mime_type:mime,expected_size,attributes:Object::new()}));
        let mut body=response.bytes_stream();let mut bytes=0u64;
        loop {let chunk=tokio::select! {_=context.cancellation.cancelled()=>Err(ImportError::new("cancelled","fetch cancelled")),chunk=body.next()=>Ok(chunk)}?;let Some(chunk)=chunk else {break;};let chunk=chunk.map_err(|e|ImportError::new("body_failed",e.to_string()))?;bytes=bytes.checked_add(chunk.len() as u64).ok_or_else(||ImportError::new("invalid_content","byte count overflow"))?;yield ContentFrame::Item(ContentEvent::FileBytes {bytes:chunk});}
        if expected_size.is_some_and(|expected|expected!=bytes) {Err(ImportError::new("invalid_content","file length mismatch"))?;}
        yield ContentFrame::Item(ContentEvent::FileEnd);
        yield ContentFrame::End(ContentSummary {items:1,bytes});
    })
}
fn disposition_filename(header: &str) -> Option<String> {
    header.split(';').skip(1).find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        if name.eq_ignore_ascii_case("filename") {
            let value = value.trim().trim_matches('"');
            (!value.is_empty()).then(|| value.to_owned())
        } else {
            None
        }
    })
}
fn accepted_mime(raw: Option<&str>, filename: Option<&str>) -> Result<String, ImportError> {
    let mime = raw
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if matches!(
        mime.as_str(),
        "text/html" | "application/xhtml+xml" | "text/event-stream"
    ) || mime.starts_with("multipart/")
    {
        return Err(ImportError::new(
            "unsupported_content",
            "response is not a regular file",
        ));
    }
    if mime.is_empty() || mime == "application/octet-stream" {
        return filename
            .and_then(|name| name.rsplit('.').next())
            .and_then(extension_mime)
            .map(str::to_owned)
            .ok_or_else(|| {
                ImportError::new(
                    "unsupported_content",
                    "generic MIME requires recognized filename extension",
                )
            });
    }
    if ["image/", "audio/", "video/", "font/"]
        .iter()
        .any(|prefix| mime.starts_with(prefix) && mime.len() > prefix.len())
        || matches!(
            mime.as_str(),
            "text/plain"
                | "text/csv"
                | "text/markdown"
                | "text/xml"
                | "application/json"
                | "application/xml"
                | "application/pdf"
                | "application/zip"
                | "application/gzip"
                | "application/x-gzip"
                | "application/x-tar"
                | "application/x-7z-compressed"
                | "application/x-rar-compressed"
                | "application/vnd.rar"
                | "application/rtf"
                | "text/rtf"
                | "application/msword"
                | "application/vnd.ms-excel"
                | "application/vnd.ms-powerpoint"
                | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                | "application/vnd.oasis.opendocument.text"
                | "application/vnd.oasis.opendocument.spreadsheet"
                | "application/vnd.oasis.opendocument.presentation"
        )
    {
        Ok(mime)
    } else {
        Err(ImportError::new(
            "unsupported_content",
            format!("unsupported MIME: {mime}"),
        ))
    }
}
fn extension_mime(extension: &str) -> Option<&'static str> {
    Some(match extension.to_ascii_lowercase().as_str() {
        "txt" => "text/plain",
        "csv" => "text/csv",
        "md" => "text/markdown",
        "json" => "application/json",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "tar" => "application/x-tar",
        "7z" => "application/x-7z-compressed",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mime_policy() {
        assert_eq!(
            accepted_mime(Some("Application/PDF; charset=UTF-8"), None).unwrap(),
            "application/pdf"
        );
        assert_eq!(accepted_mime(None, Some("report.csv")).unwrap(), "text/csv");
        for mime in [
            "text/html",
            "application/xhtml+xml",
            "text/event-stream",
            "multipart/mixed",
        ] {
            assert!(accepted_mime(Some(mime), Some("safe.pdf")).is_err());
        }
        assert!(accepted_mime(None, None).is_err());
    }
    #[test]
    fn requested_identity_is_stable_and_preserves_url_parts() {
        let request = SourceRequest {
            url: "https://EXAMPLE.com/a?b=2&a=1#part".into(),
            options: Object::new(),
        };
        assert_eq!(
            GenericUrlPlugin::identity(&request).unwrap().source,
            "https://example.com/a?b=2&a=1#part"
        );
    }
    async fn response_stream(response: &'static str) -> ContentStream {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = socket.read(&mut request).await;
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        read_url(
            reqwest::Client::new(),
            SourceRequest {
                url: format!("http://{address}/file.txt"),
                options: Object::new(),
            },
            InvocationContext {
                generation: 1,
                cancellation: semantic_jobs::CancellationToken::new(),
            },
        )
    }
    #[tokio::test]
    async fn http_empty_and_text_stream_without_writer() {
        for (response, length) in [
            (
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                0,
            ),
            (
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc",
                3,
            ),
        ] {
            let mut stream = response_stream(response).await;
            let mut validator = ContentValidator::default();
            let mut bytes = 0;
            while let Some(frame) = stream.next().await {
                let frame = frame.unwrap();
                validator.accept(&frame).unwrap();
                if let ContentFrame::Item(ContentEvent::FileBytes { bytes: chunk }) = frame {
                    bytes += chunk.len();
                }
            }
            validator.finish().unwrap();
            assert_eq!(bytes, length);
        }
    }
    #[tokio::test]
    async fn http_rejection_and_truncation() {
        for response in [
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 10\r\nConnection: close\r\n\r\nabc",
        ] {
            let mut stream = response_stream(response).await;
            let mut failed = false;
            while let Some(frame) = stream.next().await {
                match frame {
                    Err(_) => {
                        failed = true;
                        break;
                    }
                    Ok(ContentFrame::Item(ContentEvent::FileEnd)) => {
                        panic!("invalid file completed")
                    }
                    _ => {}
                }
            }
            assert!(failed);
        }
    }
    #[tokio::test]
    async fn supplied_fetch_stream_is_reused_without_network() {
        let instance = UrlInstance {
            client: reqwest::Client::new(),
            exports: vec![ImplementationDescriptor {
                export: "importer".into(),
                interface: semantic_rpc_core::interface::InterfaceRef {
                    package: PACKAGE_NAME.into(),
                    module: "v1".into(),
                    contract: None,
                    name: "Importer".into(),
                },
                package_version: "1.0.0".into(),
                fingerprint: "test".into(),
            }],
        };
        let content = Box::pin(futures_util::stream::iter(vec![
            Ok(ContentFrame::Item(ContentEvent::FileStart(FileMetadata {
                key: "file".into(),
                filename: Some("data.txt".into()),
                mime_type: "text/plain".into(),
                expected_size: Some(3),
                attributes: Object::new(),
            }))),
            Ok(ContentFrame::Item(ContentEvent::FileBytes {
                bytes: Bytes::from_static(b"abc"),
            })),
            Ok(ContentFrame::Item(ContentEvent::FileEnd)),
            Ok(ContentFrame::End(ContentSummary { items: 1, bytes: 3 })),
        ]));
        let request = FetchedRequest {
            identity: SourceIdentity {
                namespace: "caller".into(),
                source: "offline".into(),
            },
            representation: "file".into(),
            options: Object::new(),
        };
        let output = instance
            .invoke(
                ValidatedInvocation {
                    export: "importer".into(),
                    method: "import_fetched".into(),
                    arguments: vec![
                        InvocationArgument::Value(request.to_value()),
                        InvocationArgument::Stream(encode_stream(content)),
                    ],
                },
                InvocationContext {
                    generation: 1,
                    cancellation: semantic_jobs::CancellationToken::new(),
                },
            )
            .await
            .unwrap();
        let InvocationOutput::Stream(stream) = output else {
            panic!("expected stream")
        };
        let frames = decode_stream(stream).collect::<Vec<_>>().await;
        assert_eq!(frames.len(), 4);
        assert!(frames.iter().all(Result::is_ok));
        assert!(
            matches!(&frames[1],Ok(ContentFrame::Item(ContentEvent::FileBytes {bytes})) if bytes.as_ref()==b"abc")
        );
    }
}
