use brass::vdom::{self, Render};
use factordb::schema::AttrMapExt;
use semantic_ui_core::components::markdown::Markdown;

use self::note_update::NoteUpateProps;

pub mod note_create;
pub mod note_form;
pub mod note_update;

pub fn note_content(
    item: &factordb::query::select::Item,
    opts: &semantic_ui_core::EntityRenderOpts,
) -> brass::VNode {
    if opts.editable {
        match factordb::data::value::from_value_map(item.data.clone()) {
            Ok(note) => NoteUpateProps { note }.render(),
            Err(_err) => vdom::div().and("Invalid Note").build(),
        }
    } else {
        let note = if let Some(body) = item.data.get_attr::<semantic_core::base::AttrNoteBody>() {
            Markdown {
                markdown: body.clone(),
            }
            .render()
        } else {
            vdom::p_with("...").build()
        };
        note
    }
}
