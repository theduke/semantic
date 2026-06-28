use bytes::Bytes;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MimeAnalysis {
    pub declared: Option<String>,
    pub detected: Option<String>,
}

impl MimeAnalysis {
    pub fn best_effort(&self) -> Option<&str> {
        self.detected.as_deref().or(self.declared.as_deref())
    }
}

pub fn analyze_bytes(bytes: &[u8], declared: Option<&str>) -> MimeAnalysis {
    MimeAnalysis {
        declared: normalize_declared(declared),
        detected: detect_bytes(bytes),
    }
}

pub fn analyze_bytes_owned(bytes: &Bytes, declared: Option<&str>) -> MimeAnalysis {
    analyze_bytes(bytes.as_ref(), declared)
}

pub fn detect_bytes(bytes: &[u8]) -> Option<String> {
    infer::get(bytes).map(|kind| kind.mime_type().to_string())
}

pub fn normalize_declared(mime_type: Option<&str>) -> Option<String> {
    mime_type
        .and_then(|mime_type| mime_type.split(';').next())
        .map(str::trim)
        .filter(|mime_type| !mime_type.is_empty())
        .map(str::to_ascii_lowercase)
}

pub fn is_image(mime_type: Option<&str>) -> bool {
    mime_type
        .and_then(|mime_type| normalize_declared(Some(mime_type)))
        .is_some_and(|mime_type| mime_type.starts_with("image/"))
}

pub fn is_video(mime_type: Option<&str>) -> bool {
    mime_type
        .and_then(|mime_type| normalize_declared(Some(mime_type)))
        .is_some_and(|mime_type| mime_type.starts_with("video/"))
}

pub fn is_audio(mime_type: Option<&str>) -> bool {
    mime_type
        .and_then(|mime_type| normalize_declared(Some(mime_type)))
        .is_some_and(|mime_type| mime_type.starts_with("audio/"))
}
