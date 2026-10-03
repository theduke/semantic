//! Async neighborhood loading; the explorer itself stays synchronous.
use super::{
    EntityEdgeKind, EntityNodeData, ExpansionEdge, ExpansionRequest, ExpansionResult, GraphMode,
    GraphSource, entity_data, node_id,
};
use crate::{EntityTarget, UiCatalog};
use semantic_data::{
    attr::ATTR_PARENT,
    schema::{RelationMode, RelationType},
    value::Value,
};
use std::collections::BTreeMap;

/// Edge rows omit target collections. Only schema sources or unique known
/// entities can identify an endpoint; unknown targets stay non-navigable.
pub fn resolve_endpoint(
    id: &str,
    source: bool,
    relation: Option<&RelationType>,
    known: &[EntityTarget],
) -> EntityNodeData {
    if source && let Some(relation) = relation {
        return entity_data(
            EntityTarget::new(Some(relation.source_collection.clone()), id),
            None,
        );
    }
    let candidates = known
        .iter()
        .filter(|target| target.id == id)
        .map(|target| (target.collection_or_default().to_owned(), target.clone()))
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect::<Vec<_>>();
    if candidates.len() == 1 {
        entity_data(candidates[0].clone(), None)
    } else {
        EntityNodeData::Ambiguous {
            id: id.to_owned(),
            candidates,
        }
    }
}

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
                source: request.node.clone(),
                target: node_id(&target),
                kind: EntityEdgeKind::Parent,
            });
            result.nodes.push(entity_data(target, Some(object)));
        }
    }
    if request.mode != GraphMode::Hierarchy {
        let known = known
            .iter()
            .cloned()
            .chain(
                result
                    .nodes
                    .iter()
                    .filter_map(|node| node.target().cloned()),
            )
            .collect::<Vec<_>>();
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
            if request.mode == GraphMode::Both && relation.is_some_and(embedded_parent) {
                continue;
            }
            let source_target = resolve_endpoint(&row.source, true, relation, &known);
            let target = resolve_endpoint(&row.target, false, relation, &known);
            // An id-only edge query can return another collection's entity with the same id.
            if source_target.id() != request.node && target.id() != request.node {
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
            targets.insert(source_target.id(), source_target.clone());
            targets.insert(target.id(), target.clone());
            result.edges.push(ExpansionEdge {
                source: source_target.id(),
                target: target.id(),
                kind: EntityEdgeKind::Relation {
                    relation_id: row.relation,
                    label,
                },
            });
        }
        let entities = targets
            .values()
            .filter_map(|node| node.target().cloned())
            .collect::<Vec<_>>();
        result.nodes.extend(load_nodes(source, &entities).await?);
        result
            .nodes
            .extend(targets.into_values().filter(|node| node.target().is_none()));
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
        fetched: std::cell::RefCell<Vec<EntityTarget>>,
    }
    impl GraphSource for MockGraphSource {
        fn entities<'a>(
            &'a self,
            targets: &'a [EntityTarget],
        ) -> LocalBoxFuture<'a, Result<Vec<Object>, String>> {
            Box::pin(async move {
                self.fetched.borrow_mut().extend_from_slice(targets);
                Ok(targets
                    .iter()
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
    fn resolver_preserves_ambiguity_without_guessing_default() {
        assert!(resolve_endpoint("a", false, None, &[]).target().is_none());
        let known = vec![
            EntityTarget::new(Some("people".into()), "a"),
            EntityTarget::new(Some("files".into()), "a"),
        ];
        assert_eq!(resolve_endpoint("a", false, None, &known).target(), None);
        assert_eq!(
            resolve_endpoint("a", false, None, &known[..1])
                .target()
                .unwrap()
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
                .target()
                .unwrap()
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
        let source = MockGraphSource::default();
        let result = load_expansion(
            &source,
            &request,
            &UiCatalog::empty(),
            std::slice::from_ref(&root),
        )
        .await
        .unwrap();
        // The mock would return a real object for "missing" if asked. Its id
        // also exists in the default collection, but no such fetch is allowed.
        assert!(
            source
                .fetched
                .borrow()
                .iter()
                .all(|target| target.id == "root")
        );
        graph.apply_expansion(&request.node, result);
        assert_eq!(graph.model().edges().count(), 2);
        assert!(graph.model().nodes().any(|node| matches!(
            &node.data, EntityNodeData::Ambiguous { id, .. } if id == "missing"
        )));
        assert!(
            graph
                .model()
                .node(&node_id(&EntityTarget::default_collection("missing")))
                .is_none()
        );
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
