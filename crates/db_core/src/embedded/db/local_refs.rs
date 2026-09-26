//! Same-collection reference resolution for query row views.
//!
//! A multi-segment path such as `author.name` may step through a string value
//! (`author = "a1"`); the embedded query source then treats the string as the
//! id of a row in the same collection and continues the path on that row.
//!
//! Row views are handed to the executor as `'static` objects and cannot read
//! from the query snapshot themselves. Instead, a [`LocalRefResolver`] walks
//! the plan's candidate paths for each row while the snapshot is reachable,
//! fetches the referenced rows by point reads (memoised per query), and
//! attaches them to the row view as [`LocalRefTargets`]. Evaluation then
//! follows the same walk against those targets.
//!
//! Row ids equal their storage keys (writes validate that the canonical id
//! field matches the row id), so a point read by the referenced id finds the
//! row the previous full-collection lookup map found by its id field.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use semantic_data::value::{FieldPath, Object, PathSegment, Value};

use crate::catalog::LocalCollectionId;
use crate::embedded::storage::EntityReadSnapshot;
use crate::{CoreError, CoreResult};

/// Rows referenced by one row view, keyed by the referenced id.
pub(super) type LocalRefTargets = BTreeMap<String, Arc<Object>>;

/// Resolves same-collection string references for one query.
pub(super) struct LocalRefResolver {
    /// Paths the plan may evaluate that can step through a string id.
    paths: Vec<FieldPath>,
    /// Point reads already performed by this query.
    rows: Mutex<HashMap<(LocalCollectionId, String), Option<Arc<Object>>>>,
}

impl LocalRefResolver {
    /// A resolver for `plan`, or `None` when no expression of the plan can
    /// follow a string reference.
    pub(super) fn for_plan(plan: &crate::PhysicalPlan) -> Option<Self> {
        let mut paths = BTreeSet::new();
        collect_plan_paths(plan, &mut paths);
        Self::new(paths)
    }

    /// A resolver for the candidate `paths`, or `None` when none of them can
    /// follow a string reference.
    pub(super) fn new(paths: impl IntoIterator<Item = FieldPath>) -> Option<Self> {
        let mut candidates = BTreeSet::new();
        for path in paths {
            // Executors may evaluate a binding-qualified path (`t.author.name`)
            // relative to the source row (`author.name`), so every suffix is a
            // candidate too.
            let segments = path.segments();
            for start in 0..segments.len() {
                let suffix = FieldPath(segments[start..].to_vec());
                if may_follow_reference(&suffix) {
                    candidates.insert(suffix);
                }
            }
        }
        (!candidates.is_empty()).then(|| Self {
            paths: candidates.into_iter().collect(),
            rows: Mutex::new(HashMap::new()),
        })
    }

    /// Fetch the rows that evaluating the candidate paths on `object` steps
    /// through.
    pub(super) fn targets_for_row(
        &self,
        reader: &dyn EntityReadSnapshot,
        collection: LocalCollectionId,
        object: &Object,
    ) -> CoreResult<Option<Arc<LocalRefTargets>>> {
        let mut targets = LocalRefTargets::new();
        let mut error = None;
        for path in &self.paths {
            if crate::ObjectAccess::value_at_path_ref(object, path).is_some() {
                continue;
            }
            resolve_path_with_local_refs(object, path, &mut |id| {
                if let Some(target) = targets.get(id) {
                    return Some(target.clone());
                }
                match self.row(reader, collection, id) {
                    Ok(target) => {
                        let target = target?;
                        targets.insert(id.to_string(), target.clone());
                        Some(target)
                    }
                    Err(err) => {
                        error.get_or_insert(err);
                        None
                    }
                }
            });
            if let Some(err) = error.take() {
                return Err(err);
            }
        }
        Ok((!targets.is_empty()).then(|| Arc::new(targets)))
    }

    fn row(
        &self,
        reader: &dyn EntityReadSnapshot,
        collection: LocalCollectionId,
        id: &str,
    ) -> CoreResult<Option<Arc<Object>>> {
        let key = (collection, id.to_string());
        let mut rows = self
            .rows
            .lock()
            .map_err(|_| CoreError::new("local reference cache lock was poisoned"))?;
        if let Some(row) = rows.get(&key) {
            return Ok(row.clone());
        }
        let row = reader
            .get_entity(collection, id)
            .map_err(|err| CoreError::new(err.to_string()))?
            .map(|entity| Arc::new(entity.object));
        rows.insert(key, row.clone());
        Ok(row)
    }
}

/// Whether evaluating `path` can step from a string value to a field of the
/// referenced row: some segment after the first is a field name.
fn may_follow_reference(path: &FieldPath) -> bool {
    path.segments()
        .iter()
        .skip(1)
        .any(|segment| matches!(segment, PathSegment::Field(_)))
}

/// Walk `path` on `object`, treating a string value followed by a field
/// segment as the id of a same-collection row returned by `lookup`.
pub(super) fn resolve_path_with_local_refs(
    object: &Object,
    path: &FieldPath,
    lookup: &mut dyn FnMut(&str) -> Option<Arc<Object>>,
) -> Option<Value> {
    let mut current = match path.segments().first()? {
        PathSegment::Field(field) => value_from_object_with_alias_fallback(object, field)?,
        PathSegment::Index(_) => return None,
    };
    for segment in path.segments().iter().skip(1) {
        current = match (&current, segment) {
            (Value::Object(map), PathSegment::Field(field)) => {
                value_from_object_with_alias_fallback(map, field)?
            }
            (Value::List(items), PathSegment::Index(index)) => items.get(*index)?.clone(),
            // Fallback: treat string ids as same-collection refs.
            (Value::String(id), PathSegment::Field(field)) => {
                let target = lookup(id)?;
                value_from_object_with_alias_fallback(&target, field)?
            }
            _ => return None,
        };
    }
    Some(current)
}

pub(super) fn value_from_object_with_alias_fallback(object: &Object, field: &str) -> Option<Value> {
    if let Some(value) = object.get(field) {
        return Some(value.clone());
    }
    let wanted_plain = field.rsplit(':').next().unwrap_or(field);
    let mut matching = object.iter().filter(|(key, _)| {
        key.rsplit(':')
            .next()
            .is_some_and(|plain| plain == wanted_plain)
    });
    let (_, value) = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    Some(value.clone())
}

fn collect_plan_paths(plan: &crate::PhysicalPlan, paths: &mut BTreeSet<FieldPath>) {
    use crate::PhysicalPlan as P;

    match plan {
        P::Source(source) => collect_source_paths(source, paths),
        P::Values { .. } => {}
        P::Filter { input, predicate } => {
            collect_plan_paths(input, paths);
            collect_expr_paths(predicate, paths);
        }
        P::Sort { input, order_by } => {
            collect_plan_paths(input, paths);
            for field in order_by {
                collect_expr_paths(&field.expr, paths);
            }
        }
        P::Project { input, projection } => {
            collect_plan_paths(input, paths);
            collect_projection_paths(projection, paths);
        }
        P::Aggregate {
            input,
            group_by,
            projection,
            having,
        } => {
            collect_plan_paths(input, paths);
            for expr in group_by {
                collect_expr_paths(expr, paths);
            }
            collect_projection_paths(projection, paths);
            if let Some(having) = having {
                collect_expr_paths(having, paths);
            }
        }
        P::Limit {
            input,
            offset,
            limit,
        } => {
            collect_plan_paths(input, paths);
            collect_expr_paths(offset, paths);
            if let Some(limit) = limit {
                collect_expr_paths(limit, paths);
            }
        }
        P::Distinct { input } | P::Exchange { input, .. } | P::Materialize { input } => {
            collect_plan_paths(input, paths)
        }
        P::RepartitionHash {
            input,
            partition_keys,
            ..
        } => {
            collect_plan_paths(input, paths);
            for key in partition_keys {
                collect_join_key_paths(key, paths);
            }
        }
        P::Union { inputs, .. } => {
            for input in inputs {
                collect_plan_paths(input, paths);
            }
        }
        P::Join(join) => {
            collect_plan_paths(&join.left, paths);
            collect_plan_paths(&join.right, paths);
            match &join.condition {
                crate::PhysicalJoinCondition::True => {}
                crate::PhysicalJoinCondition::Predicate(expr) => collect_expr_paths(expr, paths),
                crate::PhysicalJoinCondition::Eq {
                    left,
                    right,
                    residual_predicate,
                } => {
                    collect_join_key_paths(left, paths);
                    collect_join_key_paths(right, paths);
                    if let Some(expr) = residual_predicate {
                        collect_expr_paths(expr, paths);
                    }
                }
            }
            if let Some(probe) = &join.index_probe {
                collect_field_ref_paths(&probe.field, paths);
                if let Some(expr) = &probe.residual_predicate {
                    collect_expr_paths(expr, paths);
                }
            }
        }
        P::ApplyExists {
            input, subquery, ..
        } => {
            collect_plan_paths(input, paths);
            collect_plan_paths(subquery, paths);
        }
        P::ApplyInSubquery {
            input,
            left,
            subquery,
            ..
        } => {
            collect_plan_paths(input, paths);
            collect_expr_paths(left, paths);
            collect_plan_paths(subquery, paths);
        }
    }
}

fn collect_source_paths(source: &crate::PhysicalSource, paths: &mut BTreeSet<FieldPath>) {
    match source {
        crate::PhysicalSource::Scan { .. } => {}
        crate::PhysicalSource::FilteredScan { predicate, .. } => {
            collect_expr_paths(predicate, paths)
        }
        crate::PhysicalSource::IndexLookup {
            field,
            residual_predicate,
            ..
        } => {
            collect_field_ref_paths(field, paths);
            if let Some(expr) = residual_predicate {
                collect_expr_paths(expr, paths);
            }
        }
    }
}

fn collect_projection_paths(
    projection: &[crate::PhysicalProjectionField],
    paths: &mut BTreeSet<FieldPath>,
) {
    for field in projection {
        collect_expr_paths(&field.expr, paths);
        if let Some(field_ref) = &field.field {
            collect_field_ref_paths(field_ref, paths);
        }
        paths.extend(field.source_path.iter().cloned());
        paths.extend(field.wildcard.iter().cloned());
    }
}

fn collect_join_key_paths(key: &crate::PhysicalJoinKey, paths: &mut BTreeSet<FieldPath>) {
    collect_field_ref_paths(&key.field, paths);
    paths.insert(key.source_path.clone());
}

fn collect_field_ref_paths(field: &crate::FieldRef, paths: &mut BTreeSet<FieldPath>) {
    if let crate::FieldRef::Path(path) = field {
        paths.insert(path.clone());
    }
}

fn collect_select_paths(query: &crate::SelectQuery, paths: &mut BTreeSet<FieldPath>) {
    for join in &query.joins {
        match &join.condition {
            crate::JoinCondition::OnExpr(expr) => collect_expr_paths(expr, paths),
            crate::JoinCondition::UsingFields { left, right } => {
                paths.insert(left.clone());
                paths.insert(right.clone());
            }
        }
        if let Some(expr) = &join.predicate {
            collect_expr_paths(expr, paths);
        }
    }
    for expr in query
        .predicate
        .iter()
        .chain(&query.group_by)
        .chain(&query.having)
        .chain(std::iter::once(&query.offset))
        .chain(&query.limit)
    {
        collect_expr_paths(expr, paths);
    }
    for field in &query.projection {
        collect_expr_paths(&field.expr, paths);
        paths.extend(field.wildcard.iter().cloned());
    }
    for order in &query.order_by {
        collect_expr_paths(&order.expr, paths);
    }
}

fn collect_function_arg_paths(arg: &crate::FunctionArg, paths: &mut BTreeSet<FieldPath>) {
    if let semantic_data::query::FunctionArg::Expr(expr) = arg {
        collect_expr_paths(expr, paths);
    }
}

fn collect_expr_paths(expr: &crate::Expr, paths: &mut BTreeSet<FieldPath>) {
    use crate::Expr as E;

    match expr {
        E::Operand(crate::Operand::Field(path)) => {
            paths.insert(path.clone());
        }
        E::Operand(crate::Operand::Literal(_)) => {}
        E::Unary { expr, .. } | E::IsNull { expr, .. } => collect_expr_paths(expr, paths),
        E::Binary { left, right, .. } => {
            collect_expr_paths(left, paths);
            collect_expr_paths(right, paths);
        }
        E::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            collect_expr_paths(cond, paths);
            collect_expr_paths(then_expr, paths);
            collect_expr_paths(else_expr, paths);
        }
        E::Coalesce(items) => {
            for item in items {
                collect_expr_paths(item, paths);
            }
        }
        E::Function { args, .. } => {
            for arg in args {
                collect_function_arg_paths(arg, paths);
            }
        }
        E::Aggregate { arg, .. } => collect_function_arg_paths(arg, paths),
        E::InList { expr, list, .. } => {
            collect_expr_paths(expr, paths);
            for item in list {
                collect_expr_paths(item, paths);
            }
        }
        E::Subquery(query) | E::Exists { query, .. } => collect_select_paths(query, paths),
        E::Between {
            expr, low, high, ..
        } => {
            collect_expr_paths(expr, paths);
            collect_expr_paths(low, paths);
            collect_expr_paths(high, paths);
        }
        E::PatternMatch { expr, pattern, .. } | E::RegexMatch { expr, pattern, .. } => {
            collect_expr_paths(expr, paths);
            collect_expr_paths(pattern, paths);
        }
        E::RelationExists {
            relation,
            source,
            target,
            max_depth,
            ..
        } => {
            collect_expr_paths(relation, paths);
            collect_expr_paths(source, paths);
            collect_expr_paths(target, paths);
            if let Some(max_depth) = max_depth {
                collect_expr_paths(max_depth, paths);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_paths_that_can_step_through_a_string_need_a_resolver() {
        assert!(LocalRefResolver::new([FieldPath::from_fields(["title"])]).is_none());
        assert!(
            LocalRefResolver::new([FieldPath(vec![
                PathSegment::Field("tags".into()),
                PathSegment::Index(0),
            ])])
            .is_none()
        );

        let resolver =
            LocalRefResolver::new([FieldPath::from_fields(["t", "parent", "title"])]).unwrap();
        assert_eq!(
            resolver.paths,
            vec![
                FieldPath::from_fields(["parent", "title"]),
                FieldPath::from_fields(["t", "parent", "title"]),
            ]
        );
    }

    #[test]
    fn resolves_chained_references_through_lookup() {
        let mut grand = Object::new();
        grand.insert("title", Value::String("root".into()));
        let mut parent = Object::new();
        parent.insert("parent", Value::String("grand".into()));
        let mut child = Object::new();
        child.insert("local:core:parent", Value::String("parent".into()));
        let rows = BTreeMap::from([
            ("grand".to_string(), Arc::new(grand)),
            ("parent".to_string(), Arc::new(parent)),
        ]);

        let resolved = resolve_path_with_local_refs(
            &child,
            &FieldPath::from_fields(["parent", "parent", "title"]),
            &mut |id| rows.get(id).cloned(),
        );
        assert_eq!(resolved, Some(Value::String("root".into())));
    }
}
