//! Reusable SQL builders for the canonical directory model.
//!
//! The builders are transport-agnostic: callers execute the returned SQL through
//! their own database or RPC client. Result aliases are kept stable so directory
//! rows can be consumed consistently across native and web clients.

use semantic_data::{
    attr::ATTR_TITLE,
    builtin::DEFAULT_COLLECTION,
    bundles::directory::{
        ATTR_DIRECTORY_NODE_FROM, ATTR_DIRECTORY_NODE_ORDER, DIRECTORY_CLASS_ID,
        DIRECTORY_NODE_CLASS_ID, DIRECTORY_NODE_RELATION_ID,
    },
};

/// Canonical relationship discriminator field used by directory-node entities.
pub const ATTR_RELATION_RELATION: &str = "semantic:relation:relation";

/// Canonical relationship target field used by directory-node entities.
pub const ATTR_RELATION_TO: &str = "semantic:relation:to";

/// Controls whether a query returns every matching row or one bounded page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectoryQueryPage {
    /// Return all matching rows. Use only when the complete graph is required.
    All,
    /// Return a bounded page of rows.
    Page { limit: usize, offset: usize },
}

impl DirectoryQueryPage {
    /// Creates a bounded query page.
    pub const fn new(limit: usize, offset: usize) -> Self {
        Self::Page { limit, offset }
    }
}

/// Restricts the entities returned for a directory's children.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DirectoryChildFilter {
    /// Return every entity linked from the directory.
    #[default]
    All,
    /// Return only child entities with the canonical directory class.
    Directories,
}

/// Deterministic ordering for entities linked from a directory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DirectorySort {
    /// Directory-node order, then title and entity ID.
    #[default]
    Order,
    /// Title ascending, then entity ID.
    TitleAsc,
    /// Title descending, then entity ID.
    TitleDesc,
    /// Type, title, then entity ID.
    TypeAsc,
    /// Creation time descending, then title and entity ID.
    CreatedAtDesc,
    /// Update time descending, then title and entity ID.
    UpdatedAtDesc,
    /// Entity ID ascending.
    IdAsc,
}

/// Builds a query for all directory entities, ordered by title and ID.
///
/// `DirectoryQueryPage::All` is useful when reconstructing the complete hierarchy;
/// use a bounded page for interactive lists that can be loaded incrementally.
pub fn directories_query(page: DirectoryQueryPage) -> String {
    qualified(format!(
        "SELECT d.* FROM {entities} AS d WHERE d.type IN ({directory_class}) ORDER BY d.title ASC, d.id ASC{page}",
        entities = sql_ident(DEFAULT_COLLECTION),
        directory_class = sql_string(DIRECTORY_CLASS_ID),
        page = page_clause(page),
    ))
}

/// Builds a bounded lookup for top-level directories with an exact title.
///
/// The result is limited to `max_results`; callers commonly request two rows to
/// distinguish a unique match from an ambiguous duplicate without scanning all
/// directories or links.
pub fn root_directories_named_query(title: &str, max_results: usize) -> String {
    qualified(format!(
        "SELECT d.* FROM {entities} AS d WHERE d.type IN ({directory_class}) AND (d.title = {title} OR d.{title_attr} = {title}) AND d.id NOT IN (SELECT n.{relation_to} FROM {entities} AS n WHERE n.type IN ({node_class}) AND n.{relation_relation} = {node_relation}) ORDER BY d.title ASC, d.id ASC LIMIT {max_results}",
        entities = sql_ident(DEFAULT_COLLECTION),
        directory_class = sql_string(DIRECTORY_CLASS_ID),
        title = sql_string(title),
        title_attr = sql_ident(ATTR_TITLE),
        relation_to = sql_ident(ATTR_RELATION_TO),
        node_class = sql_string(DIRECTORY_NODE_CLASS_ID),
        relation_relation = sql_ident(ATTR_RELATION_RELATION),
        node_relation = sql_string(DIRECTORY_NODE_RELATION_ID),
    ))
}

/// Builds a targeted lookup for a directory entity by ID.
pub fn directory_by_id_query(id: &str) -> String {
    qualified(format!(
        "SELECT d.* FROM {entities} AS d WHERE d.id = {id} AND d.type IN ({directory_class}) ORDER BY d.id ASC LIMIT 1",
        entities = sql_ident(DEFAULT_COLLECTION),
        id = sql_string(id),
        directory_class = sql_string(DIRECTORY_CLASS_ID),
    ))
}

/// Builds a query for canonical directory-node links.
///
/// Rows use the aliases `directory_from`, `directory_to`, and `directory_order`.
pub fn directory_links_query(page: DirectoryQueryPage) -> String {
    qualified(format!(
        "SELECT n.id AS id, n.{node_from} AS directory_from, n.{relation_to} AS directory_to, n.{node_order} AS directory_order FROM {entities} AS n WHERE n.type IN ({node_class}) AND n.{relation_relation} = {node_relation} ORDER BY n.{node_order} ASC, n.id ASC{page}",
        node_from = sql_ident(ATTR_DIRECTORY_NODE_FROM),
        relation_to = sql_ident(ATTR_RELATION_TO),
        node_order = sql_ident(ATTR_DIRECTORY_NODE_ORDER),
        entities = sql_ident(DEFAULT_COLLECTION),
        node_class = sql_string(DIRECTORY_NODE_CLASS_ID),
        relation_relation = sql_ident(ATTR_RELATION_RELATION),
        node_relation = sql_string(DIRECTORY_NODE_RELATION_ID),
        page = page_clause(page),
    ))
}

/// Builds a query for every entity linked in the directory graph.
///
/// Child fields are preserved and link metadata is exposed as `directory_from`,
/// `directory_to`, and `directory_order`.
pub fn directory_tree_items_query(page: DirectoryQueryPage) -> String {
    qualified(format!(
        "SELECT child.*, n.{node_from} AS directory_from, n.{relation_to} AS directory_to, n.{node_order} AS directory_order FROM {entities} AS n INNER JOIN {entities}._ AS child ON n.{relation_to} = child.id WHERE n.type IN ({node_class}) AND n.{relation_relation} = {node_relation} ORDER BY n.{node_order} ASC, child.title ASC, child.id ASC{page}",
        node_from = sql_ident(ATTR_DIRECTORY_NODE_FROM),
        relation_to = sql_ident(ATTR_RELATION_TO),
        node_order = sql_ident(ATTR_DIRECTORY_NODE_ORDER),
        entities = sql_ident(DEFAULT_COLLECTION),
        node_class = sql_string(DIRECTORY_NODE_CLASS_ID),
        relation_relation = sql_ident(ATTR_RELATION_RELATION),
        node_relation = sql_string(DIRECTORY_NODE_RELATION_ID),
        page = page_clause(page),
    ))
}

/// Builds a query for one directory's children.
///
/// Child fields are preserved and the link order is exposed as `directory_order`.
pub fn directory_children_query(
    parent_id: &str,
    filter: DirectoryChildFilter,
    sort: DirectorySort,
    page: DirectoryQueryPage,
) -> String {
    let child_filter = match filter {
        DirectoryChildFilter::All => String::new(),
        DirectoryChildFilter::Directories => {
            format!(" AND child.type IN ({})", sql_string(DIRECTORY_CLASS_ID))
        }
    };
    qualified(format!(
        "SELECT child.*, n.{node_order} AS directory_order FROM {entities} AS n INNER JOIN {entities}._ AS child ON n.{relation_to} = child.id WHERE n.type IN ({node_class}) AND n.{relation_relation} = {node_relation} AND n.{node_from} = {parent_id}{child_filter} ORDER BY {order_by}{page}",
        node_order = sql_ident(ATTR_DIRECTORY_NODE_ORDER),
        entities = sql_ident(DEFAULT_COLLECTION),
        relation_to = sql_ident(ATTR_RELATION_TO),
        node_class = sql_string(DIRECTORY_NODE_CLASS_ID),
        relation_relation = sql_ident(ATTR_RELATION_RELATION),
        node_relation = sql_string(DIRECTORY_NODE_RELATION_ID),
        node_from = sql_ident(ATTR_DIRECTORY_NODE_FROM),
        parent_id = sql_string(parent_id),
        order_by = child_order_by(sort),
        page = page_clause(page),
    ))
}

/// Builds a deterministic lookup for the first parent link of an entity.
///
/// The row uses the aliases `directory_from` and `directory_order`.
pub fn directory_parent_query(child_id: &str) -> String {
    qualified(format!(
        "SELECT n.id AS id, n.{node_from} AS directory_from, n.{node_order} AS directory_order FROM {entities} AS n WHERE n.type IN ({node_class}) AND n.{relation_relation} = {node_relation} AND n.{relation_to} = {child_id} ORDER BY n.{node_order} ASC, n.id ASC LIMIT 1",
        node_from = sql_ident(ATTR_DIRECTORY_NODE_FROM),
        node_order = sql_ident(ATTR_DIRECTORY_NODE_ORDER),
        entities = sql_ident(DEFAULT_COLLECTION),
        node_class = sql_string(DIRECTORY_NODE_CLASS_ID),
        relation_relation = sql_ident(ATTR_RELATION_RELATION),
        node_relation = sql_string(DIRECTORY_NODE_RELATION_ID),
        relation_to = sql_ident(ATTR_RELATION_TO),
        child_id = sql_string(child_id),
    ))
}

/// Quotes a SQL string literal, escaping embedded single quotes.
pub fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Quotes a SQL identifier when it contains characters outside `[A-Za-z0-9_]`.
pub fn sql_ident(value: &str) -> String {
    if value
        .chars()
        .all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('"', "\"\""))
    }
}

fn qualified(query: String) -> String {
    format!("{query} FORMAT qualified")
}

fn page_clause(page: DirectoryQueryPage) -> String {
    match page {
        DirectoryQueryPage::All => String::new(),
        DirectoryQueryPage::Page { limit, offset } => {
            format!(" LIMIT {limit} OFFSET {offset}")
        }
    }
}

fn child_order_by(sort: DirectorySort) -> String {
    match sort {
        DirectorySort::Order => format!(
            "n.{} ASC, child.title ASC, child.id ASC",
            sql_ident(ATTR_DIRECTORY_NODE_ORDER)
        ),
        DirectorySort::TitleAsc => "child.title ASC, child.id ASC".to_string(),
        DirectorySort::TitleDesc => "child.title DESC, child.id ASC".to_string(),
        DirectorySort::TypeAsc => "child.type ASC, child.title ASC, child.id ASC".to_string(),
        DirectorySort::CreatedAtDesc => {
            "child.created_at DESC, child.title ASC, child.id ASC".to_string()
        }
        DirectorySort::UpdatedAtDesc => {
            "child.updated_at DESC, child.title ASC, child.id ASC".to_string()
        }
        DirectorySort::IdAsc => "child.id ASC".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_handles_strings_and_identifiers() {
        assert_eq!(sql_string("a'b"), "'a''b'");
        assert_eq!(sql_ident("entities"), "entities");
        assert_eq!(sql_ident("odd\"name"), "\"odd\"\"name\"");
    }

    #[test]
    fn directory_queries_use_canonical_class_and_relation_fields() {
        let links = directory_links_query(DirectoryQueryPage::All);
        assert!(links.contains(DIRECTORY_NODE_CLASS_ID));
        assert!(links.contains(DIRECTORY_NODE_RELATION_ID));
        assert!(links.contains(&sql_ident(ATTR_DIRECTORY_NODE_FROM)));
        assert!(links.contains(&sql_ident(ATTR_RELATION_RELATION)));
        assert!(links.contains(&sql_ident(ATTR_RELATION_TO)));

        let directories = directories_query(DirectoryQueryPage::All);
        assert!(directories.contains(DIRECTORY_CLASS_ID));
    }

    #[test]
    fn targeted_root_lookup_filters_and_bounds_results() {
        let query = root_directories_named_query("Ada's files", 2);
        assert!(query.contains("d.title = 'Ada''s files'"));
        assert!(query.contains("d.id NOT IN (SELECT"));
        assert!(query.contains(DIRECTORY_NODE_RELATION_ID));
        assert!(query.contains("ORDER BY d.title ASC, d.id ASC LIMIT 2"));
    }

    #[test]
    fn child_filter_sort_and_paging_are_deterministic() {
        let query = directory_children_query(
            "parent'1",
            DirectoryChildFilter::Directories,
            DirectorySort::TitleDesc,
            DirectoryQueryPage::new(50, 100),
        );
        assert!(query.contains("child.type IN"));
        assert!(query.contains("= 'parent''1'"));
        assert!(query.contains("ORDER BY child.title DESC, child.id ASC"));
        assert!(query.contains("LIMIT 50 OFFSET 100 FORMAT qualified"));
    }

    #[test]
    fn ui_and_cli_result_aliases_are_stable() {
        let links = directory_links_query(DirectoryQueryPage::All);
        assert!(links.contains("AS directory_from"));
        assert!(links.contains("AS directory_to"));
        assert!(links.contains("AS directory_order"));

        let children = directory_children_query(
            "parent",
            DirectoryChildFilter::All,
            DirectorySort::Order,
            DirectoryQueryPage::new(1_000, 0),
        );
        assert!(children.starts_with("SELECT child.*"));
        assert!(children.contains("AS directory_order"));
        assert!(children.contains("ORDER BY n."));

        let parent = directory_parent_query("child");
        assert!(parent.contains("AS directory_from"));
        assert!(parent.contains("AS directory_order"));
    }

    #[test]
    fn unpaged_queries_do_not_emit_a_limit() {
        let query = directories_query(DirectoryQueryPage::All);
        assert!(!query.contains(" LIMIT "));
        assert!(query.ends_with("FORMAT qualified"));
    }
}
