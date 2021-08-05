pub mod persister;

use std::rc::Rc;

use brass::Callback;
use factordb::query::select::Item;

pub struct EntityFormProps {
    item: Option<Item>,
    on_valid: Callback<Item>,
}

pub type DynEntityFormRenderer = Rc<dyn Fn(&EntityFormProps) -> brass::VNode>;
