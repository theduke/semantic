//! Async neighborhood loading; the explorer itself stays synchronous.
use super::{
    EntityEdgeKind, EntityNodeData, ExpansionEdge, ExpansionRequest, ExpansionResult, GraphMode,
    GraphSource, entity_data, node_id,
};
use crate::UiCatalog;
use semantic_data::{
    attr::ATTR_PARENT,
    schema::{RelationMode, RelationType},
    value::Value,
};
use std::collections::BTreeSet;

fn embedded_parent(relation: &RelationType) -> bool {
    matches!(&relation.mode, RelationMode::Embedded { attribute } if attribute == ATTR_PARENT)
}

pub async fn load_nodes(
    source: &dyn GraphSource,
    ids: &[String],
) -> Result<Vec<EntityNodeData>, String> {
    let objects = source.entities(ids).await?;
    Ok(ids
        .iter()
        .map(|id| {
            let object = objects
                .iter()
                .find(|object| object.get("id").and_then(Value::as_str) == Some(id.as_str()))
                .cloned();
            entity_data(id.clone(), object)
        })
        .collect())
}

pub async fn load_expansion(
    source: &dyn GraphSource,
    request: &ExpansionRequest,
    catalog: &UiCatalog,
) -> Result<ExpansionResult, String> {
    let mut result = ExpansionResult::default();
    if request.mode != GraphMode::Relations {
        let objects = source
            .children(std::slice::from_ref(&request.entity_id), request.limit)
            .await?;
        for object in objects {
            let Some(id) = object.get("id").and_then(Value::as_str) else {
                continue;
            };
            result.edges.push(ExpansionEdge {
                source: request.node.clone(),
                target: node_id(id),
                kind: EntityEdgeKind::Parent,
            });
            result.nodes.push(entity_data(id.to_owned(), Some(object)));
        }
    }
    if request.mode != GraphMode::Hierarchy {
        let rows = source
            .relation_edges(std::slice::from_ref(&request.entity_id), request.limit)
            .await?;
        let mut ids = BTreeSet::new();
        for row in rows {
            let relation = catalog
                .snapshot()
                .relationships
                .iter()
                .map(|stored| &stored.relationship)
                .find(|relation| relation.id == row.relation);
            if request.mode == GraphMode::Both && relation.is_some_and(embedded_parent) {
                continue;
            }
            let source_id = node_id(&row.source);
            let target_id = node_id(&row.target);
            if source_id != request.node && target_id != request.node {
                continue;
            }
            let label = relation
                .map(|relation| {
                    relation
                        .meta
                        .title
                        .clone()
                        .unwrap_or_else(|| relation.name.clone())
                })
                .unwrap_or_else(|| row.relation.clone());
            ids.insert(row.source);
            ids.insert(row.target);
            result.edges.push(ExpansionEdge {
                source: source_id,
                target: target_id,
                kind: EntityEdgeKind::Relation {
                    relation_id: row.relation,
                    label,
                },
            });
        }
        result
            .nodes
            .extend(load_nodes(source, &ids.into_iter().collect::<Vec<_>>()).await?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EntityTarget;
    use crate::graph::{EntityGraphExplorer, ExplorerLimits, RelationEdgeRow};
    use futures::future::LocalBoxFuture;
    use semantic_data::value::Object;
    use std::collections::BTreeMap;
    #[derive(Default)]
    struct MockGraphSource {
        fetched: std::cell::RefCell<Vec<Vec<String>>>,
        objects: BTreeMap<String, Object>,
        children: Vec<Object>,
        children_requests: std::cell::RefCell<Vec<(Vec<String>, usize)>>,
        relation_requests: std::cell::RefCell<Vec<(Vec<String>, usize)>>,
    }
    impl GraphSource for MockGraphSource {
        fn entities<'a>(
            &'a self,
            targets: &'a [String],
        ) -> LocalBoxFuture<'a, Result<Vec<Object>, String>> {
            Box::pin(async move {
                self.fetched.borrow_mut().push(targets.to_vec());
                Ok(targets
                    .iter()
                    .filter_map(|target| self.objects.get(target).cloned())
                    .collect())
            })
        }
        fn children<'a>(
            &'a self,
            parents: &'a [String],
            limit: usize,
        ) -> LocalBoxFuture<'a, Result<Vec<Object>, String>> {
            Box::pin(async move {
                self.children_requests
                    .borrow_mut()
                    .push((parents.to_vec(), limit));
                Ok(self.children.iter().take(limit).cloned().collect())
            })
        }
        fn relation_edges<'a>(
            &'a self,
            ids: &'a [String],
            limit: usize,
        ) -> LocalBoxFuture<'a, Result<Vec<RelationEdgeRow>, String>> {
            Box::pin(async move {
                self.relation_requests
                    .borrow_mut()
                    .push((ids.to_vec(), limit));
                Ok(vec![
                    RelationEdgeRow {
                        relation: "friend".into(),
                        source: "root".into(),
                        target: "outgoing".into(),
                    },
                    RelationEdgeRow {
                        relation: "friend".into(),
                        source: "other".into(),
                        target: "root".into(),
                    },
                    RelationEdgeRow {
                        relation: "friend".into(),
                        source: "root".into(),
                        target: "missing".into(),
                    },
                ])
            })
        }
    }
    fn catalog(collections: &[&str], source_collection: &str) -> UiCatalog {
        use semantic_db_core::catalog::{
            LocalCollectionId, LocalRelationId, StoredCollection, StoredRelationship,
        };
        let mut snapshot = UiCatalog::empty().snapshot().clone();
        snapshot.collections = collections
            .iter()
            .enumerate()
            .map(|(index, name)| StoredCollection {
                lid: LocalCollectionId(index),
                name: (*name).into(),
                kind: None,
                integrity_mode: semantic_data::query::IntegrityMode::Permissive,
                internal: false,
                field_ids: Vec::new(),
            })
            .collect();
        snapshot.relationships.push(StoredRelationship {
            lid: LocalRelationId(0),
            relationship: RelationType {
                id: "friend".into(),
                name: "friend".into(),
                source_collection: source_collection.into(),
                mode: RelationMode::External,
                indexing_mode: semantic_data::schema::RelationIndexingMode::Enabled,
                meta: Default::default(),
            },
        });
        UiCatalog::builder(snapshot)
            .with_config(crate::ui_catalog::UiCatalogConfig::default())
            .build()
    }
    fn object(target: &EntityTarget) -> (String, Object) {
        let mut row = Object::new();
        row.insert("id", Value::String(target.id.clone()));
        row.insert(
            "semantic:title",
            Value::String(format!("Title {}", target.id)),
        );
        (target.id.clone(), row)
    }
    #[tokio::test]
    async fn relation_endpoints_use_one_default_collection_batch_including_missing() {
        let root = EntityTarget::default_collection("root");
        let outgoing = EntityTarget::default_collection("outgoing");
        let incoming = EntityTarget::default_collection("other");
        let missing = EntityTarget::default_collection("missing");
        let mut graph = EntityGraphExplorer::new(
            root.id.clone(),
            GraphMode::Relations,
            ExplorerLimits::default(),
        );
        let request = graph.begin_expand(&node_id(&root.id)).unwrap();
        let mut source = MockGraphSource {
            objects: [&root, &outgoing, &incoming]
                .into_iter()
                .map(object)
                .collect(),
            ..Default::default()
        };
        // Catalog collection metadata does not alter entity endpoint identities.
        let result = load_expansion(
            &source,
            &request,
            &catalog(&["entities", "people"], "people"),
        )
        .await
        .unwrap();
        assert_eq!(
            source.fetched.borrow().as_slice(),
            &[vec![
                missing.id.clone(),
                incoming.id.clone(),
                outgoing.id.clone(),
                root.id.clone()
            ]]
        );
        assert_eq!(
            source.relation_requests.borrow().as_slice(),
            &[(vec![root.id.clone()], request.limit)]
        );
        graph.apply_expansion(&request.node, result);
        assert_eq!(graph.model().edges().count(), 3);
        for target in [&outgoing, &incoming] {
            let node = graph.model().node(&node_id(&target.id)).unwrap();
            assert_eq!(node.data.target(), Some(target));
            assert!(node.data.object().is_some());
            assert!(graph.begin_expand(&node_id(&target.id)).is_some());
        }
        let unresolved = graph.model().node(&node_id(&missing.id)).unwrap();
        assert_eq!(unresolved.data.target(), Some(&missing));
        assert!(unresolved.data.object().is_none());
        let unresolved_id = unresolved.id.clone();
        let unresolved_parent = unresolved.layout_parent.clone();
        let edges = graph.model().edges().cloned().collect::<Vec<_>>();
        let missing_request = graph.begin_expand(&unresolved_id).unwrap();
        assert_eq!(missing_request.entity_id, missing.id);
        let (resolved_id, resolved) = object(&missing);
        source.objects.insert(resolved_id, resolved);
        let loaded = load_nodes(&source, std::slice::from_ref(&missing.id))
            .await
            .unwrap();
        assert_eq!(loaded[0].id(), unresolved_id);
        graph.set_object(&missing.id, loaded[0].object().unwrap().clone());
        assert!(
            graph
                .model()
                .node(&node_id(&missing.id))
                .unwrap()
                .data
                .object()
                .is_some()
        );
        let resolved = graph.model().node(&unresolved_id).unwrap();
        assert_eq!(resolved.id, unresolved_id);
        assert_eq!(resolved.layout_parent, unresolved_parent);
        assert_eq!(resolved.data.target(), Some(&missing));
        assert!(resolved.data.loading());
        assert_eq!(graph.model().nodes().count(), 4);
        assert_eq!(graph.model().edges().cloned().collect::<Vec<_>>(), edges);
        assert!(graph.is_expanded(&request.node));
    }

    #[tokio::test]
    async fn hierarchy_children_use_default_entities_and_preserve_request_limit() {
        let root = EntityTarget::default_collection("root");
        let child = EntityTarget::default_collection("child");
        let mut graph = EntityGraphExplorer::new(
            root.id.clone(),
            GraphMode::Hierarchy,
            ExplorerLimits::default(),
        );
        let request = graph.begin_expand(&node_id(&root.id)).unwrap();
        let source = MockGraphSource {
            children: vec![object(&child).1],
            ..Default::default()
        };
        let result = load_expansion(&source, &request, &UiCatalog::empty())
            .await
            .unwrap();
        assert_eq!(
            source.children_requests.borrow().as_slice(),
            &[(vec!["root".into()], request.limit)]
        );
        assert_eq!(result.nodes[0].target(), Some(&child));
        assert_eq!(result.edges[0].source, node_id(&root.id));
        assert_eq!(result.edges[0].target, node_id(&child.id));
        assert!(source.fetched.borrow().is_empty());
    }

    #[test]
    fn only_embedded_parent_attribute_is_filtered() {
        let mut relation = RelationType {
            id: "custom-parent".into(),
            name: "parent".into(),
            source_collection: "people".into(),
            mode: RelationMode::External,
            indexing_mode: semantic_data::schema::RelationIndexingMode::Enabled,
            meta: Default::default(),
        };
        assert!(!embedded_parent(&relation));
        relation.mode = RelationMode::Embedded {
            attribute: ATTR_PARENT.into(),
        };
        assert!(embedded_parent(&relation));
        relation.mode = RelationMode::Embedded {
            attribute: "custom:parent".into(),
        };
        assert!(!embedded_parent(&relation));
    }
}
