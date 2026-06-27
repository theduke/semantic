use std::rc::Rc;

use dioxus::prelude::Element;
use semantic_data::value::Object;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Audio,
    Video,
    File,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaRenderOptions {
    pub playing: bool,
    pub muted: bool,
    pub controls: bool,
}

impl Default for MediaRenderOptions {
    fn default() -> Self {
        Self {
            playing: false,
            muted: false,
            controls: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaRenderEvent {
    Loaded,
    Finished,
    Paused,
    Resumed,
    Failed(String),
}

pub trait MediaHandle {
    fn play(&self);
    fn pause(&self);
    fn set_muted(&self, muted: bool);
}

#[derive(Clone)]
pub struct MediaRendererRegistration {
    pub name: String,
    pub class_id: Option<String>,
    pub media_kind: MediaKind,
    pub supports_playback: bool,
    pub renderer: Rc<dyn Fn(Object, MediaRenderOptions) -> Element>,
}
