#[cfg(feature = "schema")]
fn main() {
    use semantic_core::{api, core, plugin};

    use factordb::prelude::{DataMap, ValueMap};

    let code = ts_rs::SingleFileExporter::new(true)
        // factordb
        .and::<factordb::prelude::Batch>()
        .and::<factordb::prelude::Mutate>()
        .and::<factordb::prelude::Select>()
        .and::<factordb::prelude::Id>()
        .and::<factordb::prelude::IdOrIdent>()
        .and::<factordb::prelude::Item<DataMap>>()
        .and::<factordb::prelude::Item<DataMap>>()
        .and::<factordb::prelude::Page<factordb::prelude::Item<DataMap>>>()
        .and::<factordb::prelude::Timestamp>()
        .and::<factordb::prelude::Timestamp>()
        .and::<factordb::prelude::Value>()
        .and::<factordb::prelude::Expr>()
        .and::<factordb::query::expr::UnaryOp>()
        .and::<factordb::query::expr::BinaryOp>()
        .and::<factordb::prelude::Sort>()
        .and::<factordb::schema::DbSchema>()
        .and::<factordb::schema::AttributeSchema>()
        .and::<factordb::schema::EntitySchema>()
        .and::<factordb::schema::EntityAttribute>()
        .and::<factordb::schema::Cardinality>()
        .and::<factordb::schema::IndexSchema>()
        .and::<factordb::data::ValueType>()
        .and::<factordb::data::value_type::ObjectType>()
        .and::<factordb::data::value_type::ObjectField>()
        .and::<factordb::data::value_type::MapType>()
        .and::<factordb::data::patch::Patch>()
        .and::<factordb::data::patch::PatchPath>()
        .and::<factordb::data::patch::PatchPathElem>()
        .and::<factordb::data::patch::PatchOp>()
        .and::<factordb::query::select::Join>()
        .and::<factordb::query::select::Order>()
        .and::<factordb::query::select::JoinItem<DataMap>>()
        .and::<factordb::query::mutate::Create>()
        .and::<factordb::query::mutate::Delete>()
        .and::<factordb::query::mutate::Merge>()
        .and::<factordb::query::mutate::EntityPatch>()
        .and::<factordb::query::mutate::Replace>()
        //
        .and::<api::Query>()
        .and::<api::ApiResponse<api::Reply>>()
        .and::<api::ApiError>()
        .and::<api::PluginDelete>()
        .and::<api::FileDiscardUnOptimized>()
        .and::<api::SimpleHttpRequest>()
        .and::<api::SimpleHttpResponse>()
        .and::<api::BackendConfig>()
        .and::<api::BlobInfo>()
        .and::<api::PluginTestFetch>()
        .and::<api::OptimiseVideo>()
        .and::<api::UnusedBlobsDeleted>()
        .and::<api::FileCreatePreviewImageBlob>()
        .and::<api::SemanticSchema>()
        .and::<api::BackendStatus>()
        .and::<api::DbConfig>()
        .and::<api::BackendCryptoConfig>()
        .and::<api::Job>()
        .and::<api::JobStep>()
        .and::<api::JobStatus>()
        .and::<api::ServerStatus>()
        .and::<api::OptimiseVideo>()
        .and::<api::OptimiseVideoReply>()
        .and::<api::ConvertFile>()
        .and::<plugin::FetchUrlOutput>()
        .and::<plugin::ImportJob>()
        .and::<plugin::FetchUrlJob>()
        .and::<plugin::RelatedUrl>()
        .and::<plugin::ImportOutput>()
        .and::<plugin::FetchUrlOutput>()
        .and::<core::PluginSource>()
        .and::<api::Reply>()
        .finish()
        .unwrap();

    print!("{}", code);
}

#[cfg(not(feature = "schema"))]
fn main() {
    panic!("The jsonschema feature must be enabled to generate schemas");
}
