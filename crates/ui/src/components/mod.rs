mod action_primitives;
mod copyable_code;
mod entity_explorer;
mod entity_page_header;
mod form_page;
mod global_search;
mod page_header;
mod query_builder;
mod shell;
mod upload_primitives;

pub use action_primitives::{
    ConfirmAction, ConfirmActionProps, ConfirmActionRequest, ConfirmActionVariant,
    ConfirmDangerDialog, ConfirmDangerDialogProps, IconButton, IconButtonProps, IconButtonSize,
    IconButtonVariant,
};
pub use copyable_code::{CopyRequest, CopyableCode, CopyableCodeProps};
pub use entity_explorer::{
    DataToolbar, DataToolbarProps, EntityExplorer, EntityExplorerProps, EntityResults,
    EntityResultsProps, Pagination, PaginationProps, QueryEditor, QueryEditorProps, ResultDensity,
};
pub use entity_page_header::{EntityPageHeader, EntityPageHeaderProps};
pub use form_page::{
    FormActionStatus, FormActions, FormActionsProps, FormPage, FormPageProps, UnsavedChangesPrompt,
    UnsavedChangesPromptProps,
};
pub use global_search::GlobalSearch;
pub use page_header::{PageHeader, PageHeaderProps};
pub use query_builder::{
    FilterGroup, FilterNode, FilterOperator, FilterRule, GroupCombinator, QueryField,
    QueryFieldChoice, QueryFieldKind, StructuredQuery, StructuredQueryBuilder,
    StructuredQueryBuilderProps, compile_structured_predicate, decode_structured_query,
    encode_structured_query, fields_for_collection, sql_ident, sql_string,
};
pub use shell::{AppFrame, AppFrameVariant, AppShell, PlayerShell, PrimaryNav};
pub use upload_primitives::{DropZone, DropZoneProps, JobProgress, JobProgressProps};

pub(crate) fn value_string(value: &semantic_data::value::Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("{value:?}"))
}
