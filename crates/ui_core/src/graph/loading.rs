//! Async neighborhood loading; the explorer itself stays synchronous.
use super::{
    EntityEdgeKind, EntityNodeData, ExpansionEdge, ExpansionRequest, ExpansionResult, GraphMode,
    GraphSource, entity_data, node_id,
};
use crate::{EntityTarget, UiCatalog};
use semantic_data::{
    attr::ATTR_PARENT, builtin::DEFAULT_COLLECTION, schema::RelationType, value::Value,
};
use std::collections::{BTreeMap, BTreeSet};

/// Edge rows omit collection names. Multiple known target collections are ambiguous.
pub fn resolve_endpoint(
    id: &str,
    source: bool,
    relation: Option<&RelationType>,
    known: &[EntityTarget],
) -> EntityTarget {
    if source && let Some(relation) = relation {
        return EntityTarget::new(Some(relation.source_collection.clone()), id);
    }
    let collections = known
        .iter()
        .filter(|target| target.id == id)
        .map(|target| target.collection_or_default())
        .collect::<BTreeSet<_>>();
    match collections.len() {
        0 => EntityTarget::default_collection(id),
        1 => EntityTarget::new(
            collections
                .first()
                .filter(|collection| **collection != DEFAULT_COLLECTION)
                .map(|collection| (*collection).to_owned()),
            id,
        ),
        _ => EntityTarget::new(Some("__unresolved".into()), id),
    }
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
    for (collection, targets) in groups {
        if collection == "__unresolved" {
            nodes.extend(targets.into_iter().map(|target| entity_data(target, None)));
            continue;
        }
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
    known: &[EntityTarget],
) -> Result<ExpansionResult, String> {
    let mut result = ExpansionResult::default();
    if request.mode != GraphMode::Relations {
        let objects = source
            .children(std::slice::from_ref(&request.target), request.limit)
            .await?;
        for object in objects {
            let Some(id) = object.get("id").and_then(Value::as_str) else {
                continue;
            };
            let target = EntityTarget::new(request.target.collection.clone(), id);
            result.edges.push(ExpansionEdge {
                source: request.target.clone(),
                target: target.clone(),
                kind: EntityEdgeKind::Parent,
            });
            result.nodes.push(entity_data(target, Some(object)));
        }
    }
    if request.mode != GraphMode::Hierarchy {
        let rows = source
            .relation_edges(std::slice::from_ref(&request.target.id), request.limit)
            .await?;
        let mut targets = BTreeMap::new();
        for row in rows {
            let relation = catalog
                .snapshot()
                .relationships
                .iter()
                .map(|stored| &stored.relationship)
                .find(|relation| relation.id == row.relation);
            if request.mode == GraphMode::Both
                && relation.is_some_and(|relation| {
                    relation.id == ATTR_PARENT
                        || relation.name == ATTR_PARENT
                        || relation.name == "parent"
                })
            {
                continue;
            }
            let source_target = resolve_endpoint(&row.source, true, relation, known);
            let target = resolve_endpoint(&row.target, false, relation, known);
            // An id-only edge query can return another collection's entity with the same id.
            if node_id(&source_target) != request.node && node_id(&target) != request.node {
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
            targets.insert(node_id(&source_target), source_target.clone());
            targets.insert(node_id(&target), target.clone());
            result.edges.push(ExpansionEdge {
                source: source_target,
                target,
                kind: EntityEdgeKind::Relation {
                    relation_id: row.relation,
                    label,
                },
            });
        }
        result
            .nodes
            .extend(load_nodes(source, &targets.into_values().collect::<Vec<_>>()).await?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EntityGraphExplorer, EntityNodeKind, ExplorerLimits, RelationEdgeRow};
    use futures::future::LocalBoxFuture;
    use semantic_data::value::Object;
    struct MockGraphSource;
    impl GraphSource for MockGraphSource {
        fn entities<'a>(
            &'a self,
            targets: &'a [EntityTarget],
        ) -> LocalBoxFuture<'a, Result<Vec<Object>, String>> {
            Box::pin(async move {
                Ok(targets
                    .iter()
                    .filter(|target| target.id != "missing")
                    .map(|target| {
                        let mut row = Object::new();
                        row.insert("id", Value::String(target.id.clone()));
                        row
                    })
                    .collect())
            })
        }
        fn children<'a>(
            &'a self,
            _: &'a [EntityTarget],
            _: usize,
        ) -> LocalBoxFuture<'a, Result<Vec<Object>, String>> {
            Box::pin(async { Ok(Vec::new()) })
        }
        fn relation_edges<'a>(
            &'a self,
            _: &'a [String],
            _: usize,
        ) -> LocalBoxFuture<'a, Result<Vec<RelationEdgeRow>, String>> {
            Box::pin(async {
                Ok(vec![
                    RelationEdgeRow {
                        relation: "friend".into(),
                        source: "root".into(),
                        target: "missing".into(),
                    },
                    RelationEdgeRow {
                        relation: "friend".into(),
                        source: "other".into(),
                        target: "root".into(),
                    },
                ])
            })
        }
    }
    #[test]
    fn resolver_preserves_ambiguity_and_default_fallback() {
        assert!(resolve_endpoint("a", false, None, &[]).is_default_collection());
        let known = vec![
            EntityTarget::new(Some("people".into()), "a"),
            EntityTarget::new(Some("files".into()), "a"),
        ];
        assert_eq!(
            resolve_endpoint("a", false, None, &known)
                .collection
                .as_deref(),
            Some("__unresolved")
        );
        assert_eq!(
            resolve_endpoint("a", false, None, &known[..1])
                .collection
                .as_deref(),
            Some("people")
        );
        let relation = RelationType {
            id: "friend".into(),
            name: "friend".into(),
            source_collection: "people".into(),
            mode: semantic_data::schema::RelationMode::External,
            indexing_mode: semantic_data::schema::RelationIndexingMode::Enabled,
            meta: Default::default(),
        };
        assert_eq!(
            resolve_endpoint("a", true, Some(&relation), &known)
                .collection
                .as_deref(),
            Some("people")
        );
    }
    #[tokio::test]
    async fn mock_source_loads_both_directions_and_keeps_unresolved() {
        let root = EntityTarget::default_collection("root");
        let mut graph = EntityGraphExplorer::new(
            root.clone(),
            GraphMode::Relations,
            ExplorerLimits::default(),
        );
        let request = graph.begin_expand(&node_id(&root)).unwrap();
        let result = load_expansion(
            &MockGraphSource,
            &request,
            &UiCatalog::empty(),
            &[root.clone()],
        )
        .await
        .unwrap();
        graph.apply_expansion(&request.node, result);
        assert_eq!(graph.model().edges().count(), 2);
        assert!(matches!(
            graph
                .model()
                .node(&node_id(&EntityTarget::default_collection("missing")))
                .unwrap()
                .data
                .kind,
            EntityNodeKind::Unresolved
        ));
    }
}
