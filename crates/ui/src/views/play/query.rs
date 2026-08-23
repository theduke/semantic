use semantic_data::{
    builtin::DEFAULT_COLLECTION,
    filestore::{ATTR_FILE_MEDIA_DURATION, ATTR_FILE_MIME_TYPE, ATTR_TITLE, FILE_CLASS_ID},
};

const PAGE_SIZE: usize = 1_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaylistFilter {
    pub collection: String,
    pub search: String,
    pub images: bool,
    pub audio: bool,
    pub video: bool,
    pub advanced_sql: bool,
    pub sql: String,
    pub expand_to_media: bool,
}

impl Default for PlaylistFilter {
    fn default() -> Self {
        Self {
            collection: DEFAULT_COLLECTION.to_string(),
            search: String::new(),
            images: true,
            audio: true,
            video: true,
            advanced_sql: false,
            sql: default_raw_query(DEFAULT_COLLECTION),
            expand_to_media: false,
        }
    }
}

pub fn playlist_query(
    filter: &PlaylistFilter,
    offset: usize,
) -> std::result::Result<String, String> {
    if filter.advanced_sql {
        return validate_raw_select(&filter.sql);
    }
    if filter.collection.trim().is_empty() {
        return Err("Collection is required".to_string());
    }
    let mut kinds = Vec::new();
    if filter.images {
        kinds.push("image/%");
    }
    if filter.audio {
        kinds.push("audio/%");
    }
    if filter.video {
        kinds.push("video/%");
    }
    if kinds.is_empty() {
        return Err("Select at least one media kind".to_string());
    }
    let mime_predicate = kinds
        .iter()
        .map(|kind| {
            format!(
                "e.{} LIKE {}",
                sql_ident(ATTR_FILE_MIME_TYPE),
                sql_string(kind)
            )
        })
        .collect::<Vec<_>>()
        .join(" OR ");
    let search_predicate = if filter.search.trim().is_empty() {
        String::new()
    } else {
        let pattern = sql_string(&format!("%{}%", filter.search.trim()));
        format!(
            " AND (e.id ILIKE {pattern} OR e.title ILIKE {pattern} OR e.{} ILIKE {pattern})",
            sql_ident(ATTR_TITLE),
        )
    };
    Ok(format!(
        "SELECT e.id AS id, e.type AS type, e.title AS title, e.{title} AS semantic_title, e.{mime} AS mime_type, e.{duration} AS media_duration FROM {collection} AS e WHERE e.type = {file_class} AND ({mime_predicate}){search_predicate} ORDER BY e.title ASC, e.id ASC LIMIT {PAGE_SIZE} OFFSET {offset}",
        title = sql_ident(ATTR_TITLE),
        mime = sql_ident(ATTR_FILE_MIME_TYPE),
        duration = sql_ident(ATTR_FILE_MEDIA_DURATION),
        collection = sql_ident(&filter.collection),
        file_class = sql_string(FILE_CLASS_ID),
    ))
}

pub fn page_size() -> usize {
    PAGE_SIZE
}

fn default_raw_query(collection: &str) -> String {
    format!(
        "SELECT * FROM {} WHERE type = {} ORDER BY id ASC LIMIT 100000",
        sql_ident(collection),
        sql_string(FILE_CLASS_ID),
    )
}

fn validate_raw_select(sql: &str) -> std::result::Result<String, String> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err("SQL is required".to_string());
    }
    if !trimmed.to_ascii_lowercase().starts_with("select ") {
        return Err("The playlist accepts read-only SELECT queries only".to_string());
    }
    if trimmed.trim_end_matches(';').contains(';') {
        return Err("Only one SELECT statement is allowed".to_string());
    }
    let normalized = trimmed.to_ascii_lowercase();
    if !normalized.contains(" order by ") || !normalized.contains(" limit ") {
        return Err(
            "Raw playlist SQL must include deterministic ORDER BY and a bounded LIMIT".to_string(),
        );
    }
    Ok(trimmed.trim_end_matches(';').to_string())
}

pub fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub fn sql_ident(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('"', "\"\""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_query_is_playable_stable_and_paged() {
        let query = playlist_query(&PlaylistFilter::default(), 2_000).unwrap();
        assert!(query.contains("image/%"));
        assert!(query.contains("audio/%"));
        assert!(query.contains("video/%"));
        assert!(query.contains("ORDER BY e.title ASC, e.id ASC"));
        assert!(query.ends_with("LIMIT 1000 OFFSET 2000"));
    }

    #[test]
    fn search_and_identifiers_are_escaped() {
        let mut filter = PlaylistFilter::default();
        filter.collection = "odd\"collection".to_string();
        filter.search = "O'Brien".to_string();
        let query = playlist_query(&filter, 0).unwrap();
        assert!(query.contains("\"odd\"\"collection\""));
        assert!(query.contains("%O''Brien%"));
    }

    #[test]
    fn raw_mode_rejects_writes_and_multiple_statements() {
        let mut filter = PlaylistFilter::default();
        filter.advanced_sql = true;
        filter.sql = "DELETE FROM entities".to_string();
        assert!(playlist_query(&filter, 0).is_err());
        filter.sql = "SELECT * FROM entities; DELETE FROM entities".to_string();
        assert!(playlist_query(&filter, 0).is_err());
        filter.sql = "SELECT * FROM entities".to_string();
        assert!(playlist_query(&filter, 0).is_err());
    }
}
