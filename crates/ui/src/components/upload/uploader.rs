use brass::vdom::{self, div, div_with, s};
use factordb::{AnyError, query::select::Item, schema::EntityContainer};
use semantic_core::{
    api::FileUploadMetadata,
    base::{Collection, TypedFile},
};
use semantic_ui_core::{api, loader::LoadState, RenderContextExt};

use crate::components::base::collections::collection_picker;

pub enum Msg {
    FilesAdded(Vec<web_sys::File>),
    Upload,
    Clear,
    CollectionClear,
    CollectionSelectToggle,
    CollectionSelected(Collection),
    UploadResult {
        index: usize,
        result: Result<TypedFile, AnyError>,
    },
}

struct FileItem {
    index: usize,
    file: web_sys::File,
    status: LoadState<()>,
    // progress: u32,
}

pub struct FileUploader;

struct State {
    files: Vec<FileItem>,
    uploaded_files: Vec<Item>,
    collection_finder_active: bool,
    collection: Option<Collection>,
    loading: bool,
}

brass::enable_props!(FileUploader => State);

impl State {
    fn add_file(&mut self, file: web_sys::File) {
        self.files.push(FileItem {
            index: self.files.len(),
            file,
            status: LoadState::Idle,
            // progress: 0,
        });
    }

    fn upload(&mut self, ctx: &mut brass::Context<Msg>) {
        if self.loading {
            return;
        }
        let next_file = if let Some(f) = self.files.iter_mut().find(|f| !f.status.is_success()) {
            f
        } else {
            return;
        };

        let f = api::upload_file(
            next_file.file.clone(),
            FileUploadMetadata {
                filename: Some(next_file.file.name()),
                title: None,
                collection_id: self.collection.as_ref().map(|x| x.id),
            },
        );

        let index = next_file.index;
        self.loading = true;
        let guard = ctx.run_map(f, move |res| Msg::UploadResult { index, result: res });
        next_file.status.set_loading_guarded(guard);
    }
}

impl brass::Component for State {
    type Properties = FileUploader;

    type Msg = Msg;

    fn init(_props: Self::Properties, _ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self {
            files: Vec::new(),
            uploaded_files: Vec::new(),
            collection: None,
            collection_finder_active: false,
            loading: false,
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            Msg::FilesAdded(files) => {
                files.into_iter().for_each(|f| self.add_file(f));
            }
            Msg::Upload => {
                self.upload(ctx);
            }
            Msg::Clear => {
                self.uploaded_files.clear();

                if !self.loading {
                    self.files.clear();
                }
            }
            Msg::UploadResult { index, result } => {
                self.loading = false;
                match result {
                    Ok(typed_file) => {
                        self.files.drain(index..index + 1);
                        if let Ok(map) = typed_file.clone().into_map() {
                            self.uploaded_files.push(Item::new(map));
                        }
                    }
                    Err(err) => {
                        if let Some(file) = self.files.get_mut(index) {
                            file.status.set_failed(err);
                        }
                    }
                }
            }
            Msg::CollectionClear => {
                self.collection = None;
            }
            Msg::CollectionSelectToggle => {
                self.collection_finder_active = !self.collection_finder_active;
            }
            Msg::CollectionSelected(col) => {
                self.collection = Some(col);
                self.collection_finder_active = false;
            }
        }
    }

    fn render(&self, mut ctx: brass::RenderContext<Self>) -> brass::VNode {
        let file_input = brass_bulma::FileInput {
            label: s("Select files..."),
            multi: true,
            on_change: ctx.on(|ev: web_sys::Event| {
                let input = brass::util::input_event_target(ev).unwrap();

                let files = input
                    .files()
                    .map(|list| {
                        let len = list.length();
                        (0..len)
                            .map(|index| list.get(index).unwrap())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                Msg::FilesAdded(files)
            }),
        };

        let paster = super::clipboard_reader::ClipboardReader {
            on_paste: ctx.callback_map(Msg::FilesAdded),
        };
        let paster = vdom::div().class("mt-2 mb-2").and(paster);

        let selector = div().class(s("mb-3")).and((file_input, paster));

        let collection_finder_content = if self.collection_finder_active {
            brass_bulma::box_()
                .and(brass_bulma::subtitle_4(s("Select Collection")))
                .and(collection_picker(ctx.callback_map(
                    |item: Option<Collection>| {
                        if let Some(c) = item {
                            Msg::CollectionSelected(c)
                        } else {
                            Msg::CollectionSelectToggle
                        }
                    },
                )))
                .and(
                    brass_bulma::button()
                        .and(s("Cancel"))
                        .on_click(ctx.on_simple(|| Msg::CollectionSelectToggle)),
                )
        } else {
            if let Some(col) = &self.collection {
                vdom::div()
                    .and(
                        vdom::b()
                            .class(s("pr-3"))
                            .and(format!("Uploading to collection: {}", col.title)),
                    )
                    .and(
                        brass_bulma::button()
                            .and(s("Clear"))
                            .on_click(ctx.on_simple(|| Msg::CollectionClear)),
                    )
            } else {
                vdom::div().and(
                    brass_bulma::button()
                        .and(s("Upload to collection"))
                        .on_click(ctx.on_simple(|| Msg::CollectionSelectToggle)),
                )
            }
        };
        let collection_finder = vdom::div_with(collection_finder_content).class(s("mb-3"));

        let btn_upload = brass_bulma::button()
            .and(s("Upload"))
            .attr_toggle_if(
                self.files.is_empty() || self.loading,
                brass::dom::Attr::Disabled,
            )
            .on_click(ctx.on_simple(|| Msg::Upload));
        let btn_clear = brass_bulma::button()
            .and("Clear")
            .attr_toggle_if(
                (self.uploaded_files.is_empty() && self.files.is_empty())  || self.loading,
                brass::dom::Attr::Disabled,
            )
            .on_click(ctx.on_simple(|| Msg::Clear));
        let buttons = brass_bulma::buttons().and((btn_upload, btn_clear));

        let file_list = if self.files.is_empty() {
            brass_bulma::notification(brass_bulma::Color::Default, s("Select files to upload."))
        } else {
            let items = self
                .files
                .iter()
                .map(|item| brass_bulma::box_().and(div_with(item.file.name())));

            div().class(s("mt-4")).and_iter(items)
        };

        let uploaded_files = if self.uploaded_files.is_empty() {
            vdom::div()
        } else {
            let registry = ctx.registry();
            let items = self.uploaded_files.iter().map(|item| {
                crate::components::entity::generic_entity_view(
                    item,
                    registry,
                    &semantic_ui_core::EntityRenderOpts {
                        editable: false,
                        preview: true,
                    },
                )
            });

            vdom::div()
                .class(s("mt-4"))
                .and(brass_bulma::subtitle_4(s("Uploaded Files")))
                .and_iter(items)
        };

        div()
            .and((selector, collection_finder, buttons, file_list, uploaded_files))
            .build()
    }

    fn on_property_change(
        &mut self,
        _props: Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        return false;
    }
}
