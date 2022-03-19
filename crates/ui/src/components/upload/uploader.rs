use anyhow::bail;
use brass::{
    component::{msg::MsgComponent, Context, Handle},
    dom::{
        builder::{div, span},
        ChangeEvent, ClickEvent, Render, TagBuilder, View,
    },
    signal::{
        signal::{Mutable, SignalExt},
        signal_vec::{MutableVec, SignalVecExt},
    },
};
use factordb::{
    prelude::{AttrType, AttributeDescriptor, EntityContainer, EntityDescriptor, Expr, Id, Item},
    AnyError,
};
use semantic_core::{
    api::FileUploadMetadata,
    base::{Collection, TypedFile},
};
use semantic_ui_core::{
    base::collection::collection_create,
    components::{
        autocomplete::entity_picker::entity_picker,
        entity::entity_box::EntityBox,
        loader::Loader,
        util::{
            bold, box_, button, buttons, file_input, modal::modal, notification_default,
            subtitle_4, ButtonBuilder, Cls,
        },
    },
    EntityRenderOpts,
};
use uuid::Uuid;
use wasm_bindgen::JsCast;

// use crate::components::base::collections::collection_picker;

pub enum Msg {
    FilesAdded(Vec<web_sys::File>),
    Upload,
    Clear,
    CollectionClear,
    CollectionFind,
    CollectionSelectCancel,
    CollectionSelected(Collection),
    CollectionCreateStart,
    RemoveFile(Uuid),
    UploadResult {
        id: Uuid,
        result: Result<TypedFile, AnyError>,
    },
    ClearUploaded,
}

#[derive(Clone)]
struct FileItem {
    id: Uuid,
    file: web_sys::File,
    filename: String,
    status: Loader<()>,
    size: u64,
    mime_type: String,
    // progress: u32,
}

pub struct FileUploader;

impl Render for FileUploader {
    fn render(self) -> View {
        brass::component::build_component::<State>(self)
    }
}

enum CollectionTarget {
    None,
    Selecting,
    Selected(Collection),
    Creating,
}

impl CollectionTarget {
    fn get_collection_id(&self) -> Option<Id> {
        match self {
            CollectionTarget::Selected(col) => Some(col.id),
            _ => None,
        }
    }
}

struct State {
    files: MutableVec<FileItem>,
    uploaded_files: MutableVec<Item>,
    collection: Mutable<CollectionTarget>,
    queue_length: Mutable<usize>,

    loader: Loader<()>,
}

impl State {
    fn add_file(&mut self, file: web_sys::File) {
        let mut files = self.files.lock_mut();
        files.push_cloned(FileItem {
            id: Uuid::new_v4(),
            filename: file.name(),
            size: file.size().ceil() as u64,
            mime_type: file.type_(),
            file,
            status: Loader::new_idle(),
            // progress: 0,
        });
    }

    fn upload(&mut self, ctx: &Context<Self>) -> Result<(), AnyError> {
        if self.loader.is_loading() {
            bail!("Upload already in progress");
        }

        let next_file = if let Some(f) = self
            .files
            .lock_ref()
            .iter()
            .find(|f| f.status.is_idle())
            .map(|f| f.clone())
        {
            f
        } else {
            return Ok(());
        };

        // TODO: add tags form field.

        let f = semantic_ui_core::api::upload_file(
            next_file.file.clone(),
            FileUploadMetadata {
                filename: Some(next_file.filename),
                title: None,
                collection_id: self.collection.lock_ref().get_collection_id(),
                tag_ids: Vec::new(),
            },
        );
        let id = next_file.id;
        let guard = ctx.spawn_map(f, move |res| Msg::UploadResult { id, result: res });
        next_file.status.set_loading(guard);

        Ok(())
    }
}

impl MsgComponent for State {
    type Properties = FileUploader;
    type Msg = Msg;

    fn init(_props: Self::Properties, _ctx: Context<Self>) -> Self {
        Self {
            files: MutableVec::new(),
            queue_length: Mutable::new(0),
            uploaded_files: MutableVec::new(),
            collection: Mutable::new(CollectionTarget::None),
            loader: Loader::new_idle(),
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: Context<Self>) {
        match msg {
            Msg::FilesAdded(files) => {
                let len = files.len();
                files.into_iter().for_each(|f| self.add_file(f));
                self.queue_length.replace_with(|old| *old + len);
            }
            Msg::Upload => {
                if let Err(error) = self.upload(&ctx) {
                    tracing::error!(?error, "Could not start upload");
                }
            }
            Msg::Clear => {
                self.uploaded_files.lock_mut().clear();
                self.queue_length.set(0);

                if !self.loader.is_loading() {
                    self.files.lock_mut().clear();
                }
            }
            Msg::UploadResult { id, result } => {
                self.loader.set_idle();
                match result {
                    Ok(typed_file) => {
                        {
                            let mut lock = self.files.lock_mut();
                            if let Some(index) = lock.iter().position(|f| f.id == id) {
                                lock.remove(index);
                            }
                            self.queue_length.set(lock.len());

                            if let Ok(map) = typed_file.into_map() {
                                self.uploaded_files.lock_mut().push_cloned(Item::new(map));
                            }
                        }

                        if let Err(error) = self.upload(&ctx) {
                            tracing::error!(?error, "Could not start upload");
                        }
                    }
                    Err(err) => {
                        if let Some(mut file) =
                            self.files.lock_ref().iter().find(|f| f.id == id).cloned()
                        {
                            file.status.set_result(Err(err));
                        }
                    }
                };
            }
            Msg::CollectionClear => {
                self.collection.set(CollectionTarget::None);
            }
            Msg::CollectionFind => {
                self.collection.set(CollectionTarget::Selecting);
            }
            Msg::CollectionSelectCancel => {
                self.collection.set(CollectionTarget::None);
            }
            Msg::CollectionSelected(col) => {
                self.collection.set(CollectionTarget::Selected(col));
            }
            Msg::RemoveFile(id) => {
                let mut files = self.files.lock_mut();
                files.retain(|f| f.id != id);
                self.queue_length.set(files.len());
            }
            Msg::ClearUploaded => {
                self.uploaded_files.lock_mut().clear();
            }
            Msg::CollectionCreateStart => {
                self.collection.set(CollectionTarget::Creating);
            }
        }
    }

    fn render(&mut self, ctx: Context<Self>) -> TagBuilder {
        let on_change = ctx.on(|ev: ChangeEvent| {
            let input = ev
                .current_target()
                .unwrap()
                .dyn_into::<web_sys::HtmlInputElement>()
                .unwrap();

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
        });
        let file_input = div()
            .class("mr-2")
            .and(file_input("Select files...", true, on_change));

        let paster = super::clipboard_reader::clipboard_reader(ctx.on(Msg::FilesAdded));

        let selector = box_()
            .class(Cls::IsFlex)
            .class("mb-3")
            .and((file_input, paster));

        let handle = ctx.handle();
        let collection_finder_content = self.collection.signal_ref(move |status| match status {
            CollectionTarget::None => buttons()
                .and(
                    button()
                        .and("Find Collection")
                        .on(handle.on(|_: ClickEvent| Msg::CollectionFind)),
                )
                .and(
                    button()
                        .and("Create Collection")
                        .on(handle.on(|_: ClickEvent| Msg::CollectionCreateStart)),
                ),
            CollectionTarget::Selecting => {
                let entity_filter = Expr::eq(AttrType::expr(), Collection::QUALIFIED_NAME);
                div()
                    .and(subtitle_4().and("Select Collection"))
                    .and(
                        entity_picker(
                            entity_filter,
                            handle.on_opt(|item: Item| {
                                Collection::try_from_map(item.data)
                                    .ok()
                                    .map(Msg::CollectionSelected)
                            }),
                        )
                        .class("mb-4"),
                    )
                    .and(
                        div().and(
                            ButtonBuilder::new()
                                .label("Cancel")
                                .on(handle.callback(|| Msg::CollectionSelectCancel))
                                .build(),
                        ),
                    )
            }
            CollectionTarget::Selected(col) => buttons()
                .and(ButtonBuilder::new().label(&col.title).static_().build())
                .and(
                    ButtonBuilder::new()
                        .label("Clear")
                        .on(handle.callback(|| Msg::CollectionClear))
                        .build(),
                ),
            CollectionTarget::Creating => modal(
                box_()
                    .and(subtitle_4().and("Create Collection"))
                    .and(collection_create(handle.on(Msg::CollectionSelected))),
                handle.callback(|| Msg::CollectionClear),
                false,
            ),
        });
        let collection_finder = box_()
            .class("mb-3")
            .and(div().class("mb-2").and(bold().and("Add to collection")))
            .signal(collection_finder_content);

        let btn_upload = ButtonBuilder::new()
            .label("Upload")
            .signal_disabled(self.queue_length.signal().map(|x| x < 1))
            .signal_loading(self.loader.signal_loading())
            .on(ctx.callback_msg(|| Msg::Upload))
            .build();

        let btn_clear = ButtonBuilder::new()
            .label("Clear")
            .signal_disabled(self.queue_length.signal().map(|x| x < 1))
            .on(ctx.callback_msg(|| Msg::Clear))
            .build();

        let btns = buttons().and((btn_upload, btn_clear));

        let handle = ctx.handle();
        let file_queue = div()
            .class("mt-4")
            .and(subtitle_4().and("Queue"))
            .and(btns)
            .signal_vec_with_fallback(
                self.files.signal_vec_cloned(),
                move |file| render_file_item(&handle, file).build(),
                notification_default().and("Select files to upload."),
            );

        let uploaded_vec = self.uploaded_files.signal_vec_cloned();

        let handle = ctx.handle();
        let uploaded_items = div()
            .class("mt-4")
            .and(subtitle_4().and("Uploaded Files"))
            .signal(uploaded_vec.len().map(move |len| {
                let handle = handle.clone();
                if len > 0 {
                    buttons()
                        .and(
                            ButtonBuilder::new()
                                .label(format!("Clear ({len})"))
                                .on(move || {
                                    handle.send(Msg::ClearUploaded);
                                })
                                .build(),
                        )
                        .into_view()
                } else {
                    View::Empty
                }
            }))
            .signal_vec_with_fallback(
                self.uploaded_files.signal_vec_cloned(),
                move |item| {
                    EntityBox {
                        item: item.clone(),
                        show_link: true,
                        options: EntityRenderOpts {
                            editable: false,
                            preview: true,
                        },
                        on_delete: None,
                    }
                    .render()
                    .into_node()
                    .unwrap()
                },
                notification_default().and("Nothing uploaded yet."),
            );

        div().and((selector, collection_finder, file_queue, uploaded_items))
    }
}

fn render_file_item(ctx: &Handle<State>, item: &FileItem) -> TagBuilder {
    let id = item.id;
    let btn_remove = ButtonBuilder::new()
        .label("Remove")
        .on(ctx.callback(move || Msg::RemoveFile(id)))
        .build();

    let info = div()
        .style_raw("display: flex; gap: 2rem;")
        .and(span().and(&item.filename))
        .and(span().and("Size: ").and(item.size.to_string()))
        .and(span().and("Type: ").and(&item.mime_type))
        .and(btn_remove);

    let load = item.status.signal_render(|_| View::Empty);

    box_().and(info).signal(load)
}
