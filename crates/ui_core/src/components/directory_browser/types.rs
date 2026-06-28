use dioxus::prelude::*;
use semantic_data::value::Object;

#[derive(Clone, Debug, PartialEq)]
pub struct DirectoryBrowserConfig {
    pub default_page_size: usize,
    pub page_size_options: Vec<usize>,
    pub default_tree_open: bool,
}

impl Default for DirectoryBrowserConfig {
    fn default() -> Self {
        Self {
            default_page_size: 100,
            page_size_options: vec![50, 100, 250, 500],
            default_tree_open: true,
        }
    }
}

#[derive(Clone, PartialEq, Props)]
pub struct DirectoryBrowserProps {
    #[props(default)]
    pub root: Option<String>,

    #[props(default)]
    pub config: DirectoryBrowserConfig,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BrowseViewMode {
    List,
    Icons,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DirectorySort {
    Order,
    TitleAsc,
    TitleDesc,
    TypeAsc,
    CreatedAtDesc,
    UpdatedAtDesc,
    IdAsc,
}

impl DirectorySort {
    pub(super) fn all() -> [Self; 7] {
        [
            Self::Order,
            Self::TitleAsc,
            Self::TitleDesc,
            Self::TypeAsc,
            Self::CreatedAtDesc,
            Self::UpdatedAtDesc,
            Self::IdAsc,
        ]
    }

    pub(super) fn as_value(self) -> &'static str {
        match self {
            Self::Order => "order",
            Self::TitleAsc => "title_asc",
            Self::TitleDesc => "title_desc",
            Self::TypeAsc => "type_asc",
            Self::CreatedAtDesc => "created_at_desc",
            Self::UpdatedAtDesc => "updated_at_desc",
            Self::IdAsc => "id_asc",
        }
    }

    pub(super) fn from_value(value: &str) -> Self {
        match value {
            "title_asc" => Self::TitleAsc,
            "title_desc" => Self::TitleDesc,
            "type_asc" => Self::TypeAsc,
            "created_at_desc" => Self::CreatedAtDesc,
            "updated_at_desc" => Self::UpdatedAtDesc,
            "id_asc" => Self::IdAsc,
            _ => Self::Order,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Order => "Order",
            Self::TitleAsc => "Title A-Z",
            Self::TitleDesc => "Title Z-A",
            Self::TypeAsc => "Type",
            Self::CreatedAtDesc => "Created",
            Self::UpdatedAtDesc => "Updated",
            Self::IdAsc => "ID",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct DirectoryBrowseItem {
    pub id: String,
    pub collection: String,
    pub object: Object,
    pub title: String,
    pub type_id: Option<String>,
    pub is_directory: bool,
    pub order: Option<u64>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct DirectoryBreadcrumb {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct DirectoryPage {
    pub items: Vec<DirectoryBrowseItem>,
    pub has_next: bool,
    pub breadcrumbs: Vec<DirectoryBreadcrumb>,
    pub breadcrumb_cycle: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct DirectoryTreeRow {
    pub item: DirectoryBrowseItem,
    pub depth: usize,
    pub cycle: bool,
}
