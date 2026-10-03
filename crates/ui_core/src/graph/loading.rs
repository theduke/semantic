//! Async neighborhood loading; the explorer itself stays synchronous.
use super::{
    EntityEdgeKind, EntityNodeData, ExpansionEdge, ExpansionRequest, ExpansionResult, GraphMode,
    GraphSource, entity_data, node_id,
};
use crate::{EntityTarget, UiCatalog};
use semantic_data::{
    attr::ATTR_PARENT,
    builtin::DEFAULT_COLLECTION,
    schema::{RelationMode, RelationType},
    value::Value,
};
use std::collections::{BTreeMap, BTreeSet};

fn embedded_parent(relation: &RelationType) -> bool {
    matches!(&relation.mode, RelationMode::Embedded { attribute } if attribute == ATTR_PARENT)
}

pub async fn load_nodes(
    source: &dyn GraphSource,
    targets: &[EntityTarget],
) -> Result<Vec<EntityNodeData>, String> {
    let mut groups: BTreeMap<String, Vec<EntityTarget>> = BTreeMap::new();
    for target in targets {
        groups
            .entry(target.collection_or_default().into())
            .or_default()
            .push(target.clone());
    }
    let mut nodes = Vec::new();
    for targets in groups.into_values() {
        let objects = source.entities(&targets).await?;
        for target in targets {
            let object = objects
                .iter()
                .find(|object| object.get("id").and_then(Value::as_str) == Some(target.id.as_str()))
                .cloned();
            nodes.push(entity_data(target, object));
        }
    }
    Ok(nodes)
}

pub async fn load_expansion(
    source: &dyn GraphSource,
    request: &ExpansionRequest,
    catalog: &UiCatalog,
) -> Result<ExpansionResult, String> {
    let mut result = ExpansionResult::default();
    if request.mode != GraphMode::Relations
        && request.target.collection_or_default() == DEFAULT_COLLECTION
    {
        let parent = EntityTarget::default_collection(&request.target.id);
        let objects = source.children(&[parent], request.limit).await?;
        for object in objects {
            let Some(id) = object.get("id").and_then(Value::as_str) else {
                continue;
            };
            let target = EntityTarget::default_collection(id);
            result.edges.push(ExpansionEdge {
                source: request.node.clone(),
                target: node_id(&target),
                kind: EntityEdgeKind::Parent,
            });
            result.nodes.push(entity_data(target, Some(object)));
        }
    }
    if request.mode != GraphMode::Hierarchy {
        let rows = source
            .relation_edges(std::slice::from_ref(&request.target.id), request.limit)
            .await?;
        let mut targets = BTreeSet::new();
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
            let source = EntityTarget::default_collection(&row.source);
            let target = EntityTarget::default_collection(&row.target);
            // An id-only query can also match a non-entity root with the same id.
            if node_id(&source) != request.node && node_id(&target) != request.node {
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
            targets.insert(source.id.clone());
            targets.insert(target.id.clone());
            result.edges.push(ExpansionEdge {
                source: node_id(&source),
                target: node_id(&target),
                kind: EntityEdgeKind::Relation {
                    relation_id: row.relation,
                    label,
                },
            });
        }
        let targets = targets
            .into_iter()
            .map(EntityTarget::default_collection)
            .collect::<Vec<_>>();
        result.nodes.extend(load_nodes(source, &targets).await?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EntityGraphExplorer, ExplorerLimits, RelationEdgeRow};
    use futures::future::LocalBoxFuture;
    use semantic_data::value::Object;
    #[derive(Default)]
    struct MockGraphSource {
        fetched: std::cell::RefCell<Vec<Vec<EntityTarget>>>,
        objects: BTreeMap<dxgraph::NodeId, Object>,
        children: Vec<Object>,
        children_requests: std::cell::RefCell<Vec<(Vec<EntityTarget>, usize)>>,
        relation_requests: std::cell::RefCell<Vec<(Vec<String>, usize)>>,
    }
    impl GraphSource for MockGraphSource {
        fn entities<'a>(
            &'a self,
            targets: &'a [EntityTarget],
        ) -> LocalBoxFuture<'a, Result<Vec<Object>, String>> {
            Box::pin(async move {
                self.fetched.borrow_mut().push(targets.to_vec());
                Ok(targets
                    .iter()
                    .filter_map(|target| self.objects.get(&node_id(target)).cloned())
                    .collect())
            })
        }
        fn children<'a>(
            &'a self,
            parents: &'a [EntityTarget],
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
    fn object(target: &EntityTarget) -> (dxgraph::NodeId, Object) {
        let mut row = Object::new();
        row.insert("id", Value::String(target.id.clone()));
        row.insert(
            "semantic:title",
            Value::String(format!("Title {}", target.id)),
        );
        (node_id(target), row)
    }
    #[tokio::test]
    async fn relation_endpoints_use_one_default_collection_batch_including_missing() {
        let root = EntityTarget::default_collection("root");
        let outgoing = EntityTarget::default_collection("outgoing");
        let incoming = EntityTarget::default_collection("other");
        let missing = EntityTarget::default_collection("missing");
        let mut graph = EntityGraphExplorer::new(
            root.clone(),
            GraphMode::Relations,
            ExplorerLimits::default(),
        );
        let request = graph.begin_expand(&node_id(&root)).unwrap();
        let source = MockGraphSource {
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
                missing.clone(),
                incoming.clone(),
                outgoing.clone(),
                root.clone()
            ]]
        );
        assert_eq!(
            source.relation_requests.borrow().as_slice(),
            &[(vec![root.id.clone()], request.limit)]
        );
        graph.apply_expansion(&request.node, result);
        assert_eq!(graph.model().edges().count(), 3);
        for target in [&outgoing, &incoming] {
            let node = graph.model().node(&node_id(target)).unwrap();
            assert_eq!(node.data.target(), Some(target));
            assert!(node.data.object().is_some());
            assert!(graph.begin_expand(&node_id(target)).is_some());
        }
        let unresolved = graph.model().node(&node_id(&missing)).unwrap();
        assert_eq!(unresolved.data.target(), Some(&missing));
        assert!(unresolved.data.object().is_none());
        let mut resolved = Object::new();
        resolved.insert("id", Value::String(missing.id.clone()));
        graph.set_object(&missing, resolved);
        assert!(
            graph
                .model()
                .node(&node_id(&missing))
                .unwrap()
                .data
                .object()
                .is_some()
        );
        assert_eq!(graph.model().edges().count(), 3);
        assert!(graph.is_expanded(&request.node));
    }

    #[tokio::test]
    async fn hierarchy_children_use_default_entities_and_preserve_request_limit() {
        let root = EntityTarget::default_collection("root");
        let child = EntityTarget::default_collection("child");
        let mut graph = EntityGraphExplorer::new(
            root.clone(),
            GraphMode::Hierarchy,
            ExplorerLimits::default(),
        );
        let request = graph.begin_expand(&node_id(&root)).unwrap();
        let source = MockGraphSource {
            children: vec![object(&child).1],
            ..Default::default()
        };
        let result = load_expansion(&source, &request, &UiCatalog::empty())
            .await
            .unwrap();
        assert_eq!(
            source.children_requests.borrow().as_slice(),
            &[(
                vec![EntityTarget::default_collection("root")],
                request.limit
            )]
        );
        assert_eq!(result.nodes[0].target(), Some(&child));
        assert_eq!(result.edges[0].source, node_id(&root));
        assert_eq!(result.edges[0].target, node_id(&child));
        assert!(source.fetched.borrow().is_empty());
    }

    #[tokio::test]
    async fn nondefault_root_does_not_attach_default_entities_with_the_same_raw_id() {
        let root = EntityTarget::new(Some("other_collection".into()), "root");
        let mut graph =
            EntityGraphExplorer::new(root.clone(), GraphMode::Both, ExplorerLimits::default());
        let request = graph.begin_expand(&node_id(&root)).unwrap();
        let source = MockGraphSource {
            children: vec![object(&EntityTarget::default_collection("child")).1],
            ..Default::default()
        };
        let result = load_expansion(&source, &request, &UiCatalog::empty())
            .await
            .unwrap();
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
        assert!(source.children_requests.borrow().is_empty());
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
