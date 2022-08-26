#[cfg(feature = "schema")]
fn main() {
    use semantic_core::{
        api::{self},
        core, plugin,
    };

    use factdb::DataMap;

    let code = ts_rs::SingleFileExporter::new(true)
        // factordb
        .and::<factdb::Batch>()
        .and::<factdb::Mutate>()
        .and::<factdb::Select>()
        .and::<factdb::Id>()
        .and::<factdb::IdOrIdent>()
        .and::<factdb::Item<DataMap>>()
        .and::<factdb::Item<DataMap>>()
        .and::<factdb::Page<factdb::Item<DataMap>>>()
        .and::<factdb::Timestamp>()
        .and::<factdb::Timestamp>()
        .and::<factdb::Value>()
        .and::<factdb::Expr>()
        .and::<factdb::query::expr::UnaryOp>()
        .and::<factdb::query::expr::BinaryOp>()
        .and::<factdb::Sort>()
        .and::<factdb::schema::DbSchema>()
        .and::<factdb::schema::Attribute>()
        .and::<factdb::schema::Class>()
        .and::<factdb::schema::ClassAttribute>()
        .and::<factdb::schema::Cardinality>()
        .and::<factdb::schema::IndexSchema>()
        .and::<factdb::data::ValueType>()
        .and::<factdb::data::value_type::ObjectType>()
        .and::<factdb::data::value_type::ObjectField>()
        .and::<factdb::data::value_type::MapType>()
        .and::<factdb::data::patch::Patch>()
        .and::<factdb::data::patch::PatchPath>()
        .and::<factdb::data::patch::PatchPathElem>()
        .and::<factdb::data::patch::PatchOp>()
        .and::<factdb::query::select::Join>()
        .and::<factdb::query::select::Order>()
        .and::<factdb::query::select::JoinItem<DataMap>>()
        .and::<factdb::query::mutate::Create>()
        .and::<factdb::query::mutate::Delete>()
        .and::<factdb::query::mutate::Merge>()
        .and::<factdb::query::mutate::EntityPatch>()
        .and::<factdb::query::mutate::Replace>()
        .and::<factdb::query::select::Aggregation>()
        .and::<factdb::query::select::AggregationOp>()
        .and::<factdb::data::value_type::ConstrainedRefType>()
        .and::<factdb::query::mutate::MutateSelect>()
        .and::<factdb::query::mutate::MutateSelectAction>()
        //
        .and::<api::Query>()
        .and::<semantic_core::api::TagMerge>()
        .and::<semantic_core::base::RecordEntityVisit>()
        .and::<api::QuerySql>()
        .and::<api::TagCreate>()
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
        .and::<api::JobEvent>()
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
        .and::<api::FileUploadMetadata>()
        .and::<api::FileUploadReply>()
        .finish()
        .unwrap();

    print!("{}", code);
}

#[cfg(not(feature = "schema"))]
fn main() {
    panic!("The jsonschema feature must be enabled to generate schemas");
}
