use brass::vdom::{self, Render};
use factordb::schema::AttrMapExt;
use semantic_ui_core::components::markdown::Markdown;

pub mod note_create;
pub mod note_form;
pub mod note_update;

pub fn note_content(
    item: &factordb::query::select::Item,
    _opts: &semantic_ui_core::EntityRenderOpts,
) -> brass::VNode {
    let note = if let Some(body) = item.data.get_attr::<semantics_core::base::AttrNoteBody>() {
        Markdown {
            markdown: body.clone(),
        }
        .render()
    } else {
        vdom::p_with("...").build()
    };

    note
}
