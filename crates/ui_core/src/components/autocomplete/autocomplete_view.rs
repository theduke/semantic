use std::{collections::HashSet, marker::PhantomData};

use brass::{vdom, Shared, Str};

pub struct Option<T> {
    pub value: T,
    pub label: Str,
}

pub struct AutocompleteView<T> {
    pub options: Shared<Vec<Option<T>>>,
}

type Index = usize;

enum Msg {
    Search(String),
    Selected(Index),
}

struct State<T> {
    search: String,
    selected: HashSet<Index>,
    _marker: PhantomData<T>,
}

impl<T: Clone + 'static> brass::PropComponent for State<T> {
    type Properties = AutocompleteView<T>;
    type Msg = Msg;

    fn init(_props: &Self::Properties, _ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self {
            search: String::new(),
            selected: HashSet::new(),
            _marker: PhantomData,
        }
    }

    fn update(
        &mut self,
        msg: Self::Msg,
        props: &Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) {
        match msg {
            Msg::Search(term) => {
                self.search = term;
            }
            Msg::Selected(index) => {
                let value = props.options.as_ref().get(index).unwrap();
            }
        }
    }

    fn render(
        &self,
        _props: &Self::Properties,
        _ctx: brass::RenderContext<brass::PropWrapper<Self>>,
    ) -> brass::VNode {
        vdom::div().build()
    }
}
