use brass::vdom::{div, div_with};
use factordb::{schema::EntityContainer, AnyError};
use semantic_core::{api::FileUploadMetadata, base::TypedFile};
use semantic_ui_core::{api, loader::LoadState};

pub enum Msg {
    FilesAdded(Vec<web_sys::File>),
    Upload,
    Clear,
    UploadResult {
        index: usize,
        result: Result<TypedFile, AnyError>,
    },
}

struct FileItem {
    index: usize,
    file: web_sys::File,
    status: LoadState<TypedFile>,
    // progress: u32,
}

pub struct FileUploader;

struct State {
    files: Vec<FileItem>,
    collection_name: Option<String>,
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
            collection_name: None,
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
                self.files.clear();
            }
            Msg::UploadResult { index, result } => {
                if let Some(file) = self.files.get_mut(index) {
                    file.status.set_result(result);
                    self.loading = false;

                    if self.files.len() - 1 > index {
                        self.upload(ctx);
                    }
                }
            }
        }
    }

    fn render(&self, mut ctx: brass::RenderContext<Self>) -> brass::VNode {
        let file_input = brass_bulma::FileInput {
            label: "Choose files...".into(),
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

        let selector = div().and(file_input).class("mb-3");

        let btn_upload = brass_bulma::button()
            .and("Upload")
            .attr_toggle_if(
                self.files.is_empty() || self.loading,
                brass::dom::Attr::Disabled,
            )
            .on_click(ctx.on_simple(|| Msg::Upload));
        let btn_clear = brass_bulma::button()
            .and("Clear")
            .attr_toggle_if(
                self.files.is_empty() || self.loading,
                brass::dom::Attr::Disabled,
            )
            .on_click(ctx.on_simple(|| Msg::Clear));
        let paster = super::clipboard_reader::ClipboardReader {
            on_paste: ctx.callback_map(Msg::FilesAdded),
        };
        let buttons = brass_bulma::buttons().and((btn_upload, btn_clear, paster));

        let file_list = if self.files.is_empty() {
            brass_bulma::notification(brass_bulma::Color::Default, "Select files to upload.")
        } else {
            let items = self.files.iter().map(|item| {
                brass_bulma::box_().and((
                    div_with(item.file.name()),
                    div_with(item.status.render(|file| {
                        brass::vdom::component::<crate::components::router::Link>(
                            crate::components::router::LinkProps {
                                route: semantic_ui_core::routing::Route::Entity(file.id().into()),
                                text: "Show File".into(),
                                class: Some("button is-success".into()),
                            },
                        )
                    })),
                ))
            });

            div().class("mt-4").and_iter(items)
        };

        div().and((selector, buttons, file_list)).build()
    }

    fn on_property_change(
        &mut self,
        _props: Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        return false;
    }
}
