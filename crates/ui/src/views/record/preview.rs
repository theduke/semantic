use std::rc::Rc;

use super::CompletedRecording;

/// Keeps a browser object URL alive exactly as long as its audio preview.
#[derive(Clone)]
pub(super) struct PlaybackUrl(Rc<UrlOwner>);

struct UrlOwner(String);

impl PlaybackUrl {
    pub fn new(recording: &CompletedRecording) -> Result<Self, String> {
        #[cfg(target_arch = "wasm32")]
        let url = {
            let parts = js_sys::Array::new();
            parts.push(&js_sys::Uint8Array::from(recording.bytes.as_ref()));
            let options = web_sys::BlobPropertyBag::new();
            options.set_type(&recording.mime_type);
            let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)
                .map_err(|_| {
                    "Could not prepare the preview. You can still upload your capture.".to_string()
                })?;
            web_sys::Url::create_object_url_with_blob(&blob).map_err(|_| {
                "Could not create a preview. You can still upload your capture.".to_string()
            })?
        };
        // Desktop bytes are already loaded for upload. A media data URL works in
        // the WebView without JavaScript or exposing a temporary filesystem path.
        #[cfg(not(target_arch = "wasm32"))]
        let url = {
            use base64::Engine as _;
            format!(
                "data:{};base64,{}",
                recording.mime_type,
                base64::engine::general_purpose::STANDARD.encode(&recording.bytes)
            )
        };
        Ok(Self(Rc::new(UrlOwner(url))))
    }

    pub fn as_str(&self) -> &str {
        &self.0.0
    }
}

impl Drop for UrlOwner {
    fn drop(&mut self) {
        #[cfg(target_arch = "wasm32")]
        let _ = web_sys::Url::revoke_object_url(&self.0);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn desktop_preview_preserves_mime_and_audio_bytes() {
        let recording = CompletedRecording {
            bytes: bytes::Bytes::from_static(b"audio"),
            extension: "mp3",
            mime_type: "audio/mpeg".to_string(),
        };
        let url = PlaybackUrl::new(&recording).unwrap();
        assert_eq!(url.as_str(), "data:audio/mpeg;base64,YXVkaW8=");
    }
}
