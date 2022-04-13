use brass::dom::{builder::div, Tag, TagBuilder};
use bytesize::ByteSize;
use semantic_core::api::UnusedBlobsDeleted;
use semantic_ui_core::{
    components::{
        loader::Loader,
        util::{box_, notification_success, subtitle_4, table, title_2, ButtonBuilder, Color},
    },
    context,
};

pub fn blob_cleanup() -> TagBuilder {
    let loader = Loader::new_spawn(async move { context::api().find_unused_blobs().await });
    let loader_delete = Loader::<UnusedBlobsDeleted>::new_idle();

    let loader_delete2 = loader_delete.clone();
    let content = loader.signal_render(move |blobs| {
        div()
            .and(subtitle_4().and(format!("Found {} unused blobs", blobs.len())))
            .and(table().and_iter(blobs.iter().map(|b| {
                Tag::Tr
                    .new()
                    .and(Tag::Td.new().and(&b.key))
                    .and(Tag::Td.new().and(b.size.to_string()))
            })))
            .bind(loader_delete.clone())
            .and(
                ButtonBuilder::new()
                    .label("Delete all")
                    .color(Color::Danger)
                    .on({
                        let del = loader_delete.clone();
                        move || {
                            if del.is_loading() {
                                return;
                            }
                            del.spawn(async { context::api().delete_unused_blobs().await })
                        }
                    })
                    .signal_loading(loader_delete.signal_loading())
                    .build(),
            )
            .signal(loader_delete.signal_render(|out| {
                notification_success()
                    .and(format!(
                        "Deleted {} blobs with a size of {}.",
                        out.count,
                        ByteSize(out.reclaimed_size)
                    ))
                    .into_view()
            }))
            .into_view()
    });

    box_()
        .bind(loader_delete2)
        .bind(loader)
        .and(title_2().and("Blobs"))
        .signal(content)
}
