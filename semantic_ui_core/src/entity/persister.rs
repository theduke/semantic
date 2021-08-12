use std::rc::Rc;

use brass::Callback;
use factordb::query::select::Item;

use crate::{loader::LoadState, ContextExt};

pub struct FormValid {
    pub item: Item,
    pub mutation: factordb::query::mutate::BatchUpdate,
}

pub struct EntityFormProps {
    pub item: Option<Item>,
    pub on_submit: Callback<FormValid>,
    pub is_loading: bool,
    pub error: Option<String>,
}

pub type DynEntityFormRenderer = Rc<dyn Fn(&EntityFormProps) -> brass::VNode>;

pub enum Msg {
    // Valid(FormValid),
    Submit(FormValid),
    Cancel,
    Loaded(Result<(), factordb::AnyError>),
}

pub struct EntityPersister {
    pub item: Option<Item>,
    pub renderer: DynEntityFormRenderer,
    pub on_complete: Callback<Item>,
    pub on_cancel: Option<Callback<()>>,
}

struct State {
    props: EntityPersister,
    valid_data: Option<FormValid>,
    callback: Callback<FormValid>,
    loader: LoadState<()>,
}

brass::enable_props!(EntityPersister => State);

impl brass::Component for State {
    type Properties = EntityPersister;
    type Msg = Msg;

    fn init(props: Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self {
            props,
            valid_data: None,
            callback: ctx.callback_map(|data| Msg::Submit(data)),
            loader: LoadState::Idle,
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            // Msg::Valid(valid) => {
            //     self.valid_data = Some(valid);
            // }
            Msg::Submit(valid) => {
                self.valid_data = Some(valid);
                if let Some(data) = &self.valid_data {
                    let api = crate::api::api();
                    let mutation = data.mutation.clone();
                    let f = async move { api.batch(mutation).await };
                    let guard = ctx.run_map(f, Msg::Loaded);
                    self.loader.set_loading_guarded(guard);
                }
            }
            Msg::Cancel => {
                if let Some(cb) = &self.props.on_cancel {
                    cb.send(());
                }
            }
            Msg::Loaded(res) => {
                if res.is_ok() {
                    self.props
                        .on_complete
                        .send(self.valid_data.as_ref().unwrap().item.clone());
                }
                self.loader.set_result(res);
            }
        }
    }

    fn render(&self, _ctx: brass::RenderContext<Self>) -> brass::VNode {
        let form = (self.props.renderer)(&EntityFormProps {
            item: self.props.item.clone(),
            on_submit: self.callback.clone(),
            is_loading: self.loader.is_loading(),
            error: self.loader.as_error().map(|x| x.to_string()),
        });

        // let is_loading = self.loader.is_loading();

        // let submit = brass_bulma::button_medium()
        //     .and("Submit")
        //     .attr_toggle_if(is_loading, brass::dom::Attr::Disabled)
        //     .on(
        //         brass::dom::Event::Click,
        //         _ctx.on_simple(|| Msg::Submit),
        //     );

        // let cancel = if self.props.on_cancel.is_some() {
        //     brass_bulma::button_medium()
        //         .and("Cancel")
        //         .attr_toggle_if(is_loading, brass::dom::Attr::Disabled)
        //         .on(
        //             brass::dom::Event::Click,
        //             _ctx.on_simple(|| Msg::Cancel),
        //         )
        //         .build()
        // } else {
        //     vdom::VNode::Empty
        // };

        // let actions = vdom::div_with((submit, cancel));

        brass::vdom::div().and(form).build()
    }

    fn on_property_change(
        &mut self,
        props: Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        self.props = props;
        true
    }
}
