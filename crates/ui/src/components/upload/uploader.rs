use brass::{
    component::{msg::MsgComponent, Context, Handle},
    dom::{
        builder::{div, span},
        ChangeEvent, ClickEvent, Render, Tag, TagBuilder,
    },
    signal::{signal::Mutable, signal_vec::MutableVec},
};
use factordb::{
    query::{expr::Expr, select::Item},
    schema::{builtin::AttrType, AttributeDescriptor, EntityContainer, EntityDescriptor},
    AnyError, Id,
};
use semantic_core::{
    api::FileUploadMetadata,
    base::{Collection, TypedFile},
};
use semantic_ui_core::{
    components::{
        autocomplete::entity_picker::entity_picker,
        entity::entity_view::EntityView,
        loader::{LoadState, Loader},
        util::{
            box_, button, buttons, file_input, notification_default, notification_error,
            subtitle_4, ButtonBuilder, Cls,
        },
    },
    context,
};
use wasm_bindgen::JsCast;

// use crate::components::base::collections::collection_picker;

type Index = usize;

pub enum Msg {
    FilesAdded(Vec<web_sys::File>),
    Upload,
    Clear,
    CollectionClear,
    CollectionSelectStart,
    CollectionSelectCancel,
    CollectionSelected(Collection),
    RemoveFile(Index),
    UploadResult {
        index: usize,
        result: Result<TypedFile, AnyError>,
    },
}

#[derive(Clone)]
struct FileItem {
    index: Index,
    file: web_sys::File,
    filename: String,
    status: LoadState<()>,
    size: u64,
    mime_type: String,
    // progress: u32,
}

pub struct FileUploader;

impl Render for FileUploader {
    fn render(self) -> TagBuilder {
        brass::component::build_component::<State>(self)
    }
}

enum CollectionTarget {
    None,
    Selecting,
    Selected(Collection),
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

    loader: Loader<()>,
}

impl State {
    fn add_file(&mut self, file: web_sys::File) {
        let mut files = self.files.lock_mut();
        files.push_cloned(FileItem {
            index: files.len(),
            filename: file.name(),
            size: file.size().ceil() as u64,
            mime_type: file.type_(),
            file,
            status: LoadState::Idle,
            // progress: 0,
        });
    }

    fn upload(&mut self, ctx: &Context<Self>) {
        if self.loader.is_loading() {
            return;
        }

        let mut next_file = if let Some(f) = self
            .files
            .lock_ref()
            .iter()
            .find(|f| f.status.is_idle())
            .map(|f| f.clone())
        {
            f
        } else {
            return;
        };

        let f = semantic_ui_core::api::upload_file(
            next_file.file.clone(),
            FileUploadMetadata {
                filename: Some(next_file.file.name()),
                title: None,
                collection_id: self.collection.lock_ref().get_collection_id(),
            },
        );
        let index = next_file.index;
        let guard = ctx.spawn_map(f, move |res| Msg::UploadResult { index, result: res });

        next_file.status.set_loading_guarded(guard);
        let mut files = self.files.lock_mut();
        files.insert_cloned(index, next_file);
        files.remove(index + 1);
    }
}

impl MsgComponent for State {
    type Properties = FileUploader;
    type Msg = Msg;

    fn init(_props: Self::Properties, _ctx: Context<Self>) -> Self {
        Self {
            files: MutableVec::new(),
            uploaded_files: MutableVec::new(),
            collection: Mutable::new(CollectionTarget::None),
            loader: Loader::new_idle(),
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: Context<Self>) {
        match msg {
            Msg::FilesAdded(files) => {
                files.into_iter().for_each(|f| self.add_file(f));
            }
            Msg::Upload => {
                self.upload(&ctx);
            }
            Msg::Clear => {
                self.uploaded_files.lock_mut().clear();

                if !self.loader.is_loading() {
                    self.files.lock_mut().clear();
                }
            }
            Msg::UploadResult { index, result } => {
                self.loader.set_idle();
                match result {
                    Ok(typed_file) => {
                        self.files.lock_mut().drain(index..index + 1);
                        if let Ok(map) = typed_file.into_map() {
                            self.uploaded_files.lock_mut().push_cloned(Item::new(map));
                        }
                    }
                    Err(err) => {
                        if let Some(mut file) = self.files.lock_ref().get(index).cloned() {
                            file.status.set_failed(err);
                            let mut files = self.files.lock_mut();
                            files.insert_cloned(index, file);
                            files.remove(index + 1);
                        }
                    }
                };
            }
            Msg::CollectionClear => {
                self.collection.set(CollectionTarget::None);
            }
            Msg::CollectionSelectStart => {
                self.collection.set(CollectionTarget::Selecting);
            }
            Msg::CollectionSelectCancel => {
                self.collection.set(CollectionTarget::None);
            }
            Msg::CollectionSelected(col) => {
                self.collection.set(CollectionTarget::Selected(col));
            }
            Msg::RemoveFile(index) => {
                let mut files = self.files.lock_mut();
                if index < files.len() {
                    files.remove(index);
                }
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
            CollectionTarget::None => div().and(
                button()
                    .and("Upload to collection")
                    .on(handle.on(|_: ClickEvent| Msg::CollectionSelectStart)),
            ),
            CollectionTarget::Selecting => {
                let entity_filter = Expr::eq(AttrType::expr(), Collection::QUALIFIED_NAME);
                box_()
                    .and(subtitle_4().and("Select Collection"))
                    .and(entity_picker(
                        entity_filter,
                        handle.on_opt(|item: Item| {
                            Collection::try_from_map(item.data)
                                .ok()
                                .map(Msg::CollectionSelected)
                        }),
                    ))
                    .and(
                        ButtonBuilder::new()
                            .label("Cancel")
                            .on(handle.callback(|| Msg::CollectionSelectCancel))
                            .build(),
                    )
            }
            CollectionTarget::Selected(col) => div()
                .and(
                    Tag::B
                        .new()
                        .class("pr-3")
                        .and(format!("Uploading to collection: {}", col.title)),
                )
                .and(
                    button()
                        .and("Clear")
                        .on(handle.on(|_: ClickEvent| Msg::CollectionSelectCancel)),
                ),
        });
        let collection_finder = div().child_signal(collection_finder_content).class("mb-3");

        let btn_upload = ButtonBuilder::new()
            .label("Upload")
            // TODO: disable button if no files available.
            .signal_loading(self.loader.signal_loading())
            .on(ctx.callback_msg(|| Msg::Upload))
            .build();

        let btn_clear = ButtonBuilder::new()
            .label("Clear")
            // TODO: disabled state!
            // .attr_toggle_if(
            //     (self.uploaded_files.is_empty() && self.files.is_empty()) || self.loading,
            //     brass::dom::Attr::Disabled,
            // )
            .on(ctx.callback_msg(|| Msg::Clear))
            .build();

        let buttons = buttons().and((btn_upload, btn_clear));

        let handle = ctx.handle();
        let file_queue = div().class("mt-4").children_signal_with_fallback(
            self.files.signal_vec_cloned(),
            move |file| render_file_item(&handle, file).build(),
            notification_default().and("Select files to upload."),
        );

        let registry = context::registry();
        let uploaded_items = div()
            .class("mt-4")
            .and(subtitle_4().and("Uploaded Files"))
            .children_signal_with_fallback(
                self.uploaded_files.signal_vec_cloned(),
                move |item| {
                    EntityView::from_item(
                        item,
                        &registry,
                        &semantic_ui_core::EntityRenderOpts {
                            editable: false,
                            preview: true,
                        },
                    )
                    .render()
                    .build()
                },
                notification_default().and("Nothing uploaded yet."),
            );

        div().and((
            selector,
            collection_finder,
            buttons,
            file_queue,
            uploaded_items,
        ))
    }
}

fn render_file_item(ctx: &Handle<State>, item: &FileItem) -> TagBuilder {
    let index = item.index;
    let btn_remove = ButtonBuilder::new()
        .label("Remove")
        .on(ctx.callback(move || Msg::RemoveFile(index)))
        .build();

    let info = div()
        .style_raw("display: flex; gap: 2rem;")
        .and(span().and(&item.filename))
        .and(span().and("Size: ").and(item.size.to_string()))
        .and(span().and("Type: ").and(&item.mime_type))
        .and(btn_remove);

    let error = item
        .status
        .as_error()
        .map(|err| notification_error().and(err.to_string()).class("mt-3"));
    box_().and(info).and(error)
}
