use std::rc::Rc;

use brass::{
    dom::{builder::div, TagBuilder},
    signal::signal::{Mutable, SignalExt},
};
use factordb::{
    query::{mutate::Mutate, select::Item},
    schema::EntityContainer,
    Id,
};
use semantic_core::base::{Note, TextFormat};

use crate::{
    components::{
        form::{self, FormLoadFuture},
        markdown::markdown_view,
        util::{
            form_field_input, form_field_textarea, notification_error, title_2, ButtonBuilder, Cls,
            FormRenderer,
        },
    },
    context::{api, router},
    routing::Route,
    validate::StringRequired,
    EntityRenderOpts,
};

#[derive(Clone)]
pub struct Values {
    pub title: String,
    pub body: String,
}

impl Values {
    fn apply(self, note: &mut Note) {
        note.title = self.title;
        note.body = self.body;
    }
}

pub fn note_form(
    note: Note,
    on_submit_async: impl Fn(Values) -> FormLoadFuture + 'static,
) -> TagBuilder {
    form::Form::new(Values {
        title: note.title,
        body: note.body,
    })
    .on_submit_async(move |values| on_submit_async(values.clone()))
    .render(move |handle| {
        let title = form_field_input(
            "Title",
            handle.field_validated(|v| &mut v.title, StringRequired),
        );
        let body = form_field_textarea(
            "Body",
            handle.field_validated(|v| &mut v.body, StringRequired),
            5,
            true,
        );

        FormRenderer::new(handle.clone())
            .and(title)
            .and(body)
            .buttons_submit("Save")
    })
}

pub fn note_create(on_created: impl Fn(Note) + 'static) -> TagBuilder {
    let on_created = Rc::new(on_created);
    let note = Note {
        id: Id::random(),
        title: String::new(),
        body: String::new(),
        format: TextFormat::Markdown,
        extra: Default::default(),
    };
    note_form(note.clone(), move |values| {
        let on_created = on_created.clone();
        let mut note = note.clone();
        Box::pin(async move {
            note.title = values.title;
            note.body = values.body;

            api().entity_create(note.clone()).await?;
            on_created(note);

            Ok(())
        })
    })
}

pub fn note_edit(note: Note, on_saved: impl Fn(Note) + 'static) -> TagBuilder {
    let on_saved = Rc::new(on_saved);
    note_form(note.clone(), move |values| {
        let mut new_note = note.clone();
        values.apply(&mut new_note);
        let on_saved = on_saved.clone();
        let patch_res = Note::build_patch(&note, &new_note);

        Box::pin(async move {
            let patch = patch_res?;

            if patch.0.is_empty() {
                on_saved(new_note);
                Ok(())
            } else {
                api().mutate(Mutate::patch(new_note.id, patch)).await?;
                on_saved(new_note);
                Ok(())
            }
        })
    })
}

pub fn note_create_page(_item: &Item, _opts: &EntityRenderOpts) -> TagBuilder {
    note_create(|note| {
        router().goto(Route::Entity(note.id.into()));
    })
}

pub fn note_view_immutable(note: &Note) -> TagBuilder {
    div()
        .and(title_2().and(&note.title))
        .and(div().class(Cls::Content).and(markdown_view(&note.body)))
}

pub fn note_view(note: Note, opts: &EntityRenderOpts) -> TagBuilder {
    let mutable_note = Mutable::new(note);

    if opts.editable {
        let editing = Mutable::new(false);

        div().signal(editing.signal().map(move |is_editing| {
            if is_editing {
                let editing = editing.clone();
                let mutable_note = mutable_note.clone();
                note_edit(mutable_note.get_cloned(), move |note| {
                    // TODO: update note (needs support in showing logic)
                    editing.set(false);
                    mutable_note.set(note);
                })
            } else {
                let editing = editing.clone();
                let edit_btn = ButtonBuilder::new()
                    .label("Edit")
                    .on(move || {
                        editing.set(true);
                    })
                    .build();
                div()
                    .and(edit_btn)
                    .and(note_view_immutable(&mutable_note.get_cloned()))
            }
        }))
    } else {
        note_view_immutable(&mutable_note.get_cloned())
    }
}

pub fn note_content(item: &Item, opts: &EntityRenderOpts) -> TagBuilder {
    if let Ok(col) = Note::try_from_map(item.data.clone()) {
        note_view(col, opts)
    } else {
        notification_error().and("Item is not a collection")
    }
}
