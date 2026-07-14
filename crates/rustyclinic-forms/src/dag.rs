//! Dependency DAG for form expressions.
//!
//! Tracks which fields affect which other fields' expressions, enabling
//! minimal re-evaluation when a field value changes.

use std::collections::{HashMap, HashSet, VecDeque};

use thiserror::Error;

use crate::definition::FormItem;

/// Errors related to the dependency graph.
#[derive(Debug, Error)]
pub enum DagError {
    #[error("circular dependency detected involving fields: {fields:?}")]
    CircularDependency { fields: Vec<String> },
}

/// A directed acyclic graph tracking field dependencies.
///
/// Maps each field to the set of fields whose expressions reference it.
/// When field A changes, `affected_by("A")` returns all fields that need
/// re-evaluation.
#[derive(Debug, Clone)]
pub struct DependencyDag {
    /// source_field -> set of dependent fields (fields whose expressions reference source_field)
    dependents: HashMap<String, Vec<String>>,
    /// All known field link_ids.
    all_fields: HashSet<String>,
}

impl DependencyDag {
    /// Build the dependency DAG from a list of form items.
    ///
    /// Walks all expressions (enable_when, computed_value, validation) to
    /// extract field references and build the graph.
    ///
    /// Returns an error if a circular dependency is detected.
    pub fn build(items: &[FormItem]) -> Result<Self, DagError> {
        let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
        let mut all_fields = HashSet::new();

        Self::collect_dependencies(items, &mut dependents, &mut all_fields);

        let dag = Self {
            dependents,
            all_fields,
        };

        dag.check_cycles()?;

        Ok(dag)
    }

    fn collect_dependencies(
        items: &[FormItem],
        dependents: &mut HashMap<String, Vec<String>>,
        all_fields: &mut HashSet<String>,
    ) {
        for item in items {
            all_fields.insert(item.link_id.clone());

            // Collect field refs from enable_when
            if let Some(expr) = &item.enable_when {
                let refs = expr.field_references();
                for ref_id in refs {
                    // Skip self-references (a field's enable_when referencing itself is not a dependency)
                    if ref_id != item.link_id {
                        dependents
                            .entry(ref_id)
                            .or_default()
                            .push(item.link_id.clone());
                    }
                }
            }

            // Collect field refs from computed_value
            if let Some(expr) = &item.computed_value {
                let refs = expr.field_references();
                for ref_id in refs {
                    // Skip self-references
                    if ref_id != item.link_id {
                        dependents
                            .entry(ref_id)
                            .or_default()
                            .push(item.link_id.clone());
                    }
                }
            }

            // Collect field refs from validation rules
            // Note: validation rules commonly reference their own field (e.g., weight > 0
            // on the weight field). These self-references are NOT dependency edges — they
            // just need re-evaluation when the field itself changes, which always happens.
            for rule in &item.validation {
                let refs = rule.expression.field_references();
                for ref_id in refs {
                    if ref_id != item.link_id {
                        dependents
                            .entry(ref_id)
                            .or_default()
                            .push(item.link_id.clone());
                    }
                }
            }

            // Recurse into nested items
            if !item.items.is_empty() {
                Self::collect_dependencies(&item.items, dependents, all_fields);
            }
        }
    }

    /// Check for circular dependencies using topological sort.
    fn check_cycles(&self) -> Result<(), DagError> {
        // Build adjacency list for all nodes that have edges
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();

        // Initialize all known fields
        for field in &self.all_fields {
            in_degree.entry(field.as_str()).or_insert(0);
            adj.entry(field.as_str()).or_default();
        }

        // Add edges
        for (source, targets) in &self.dependents {
            for target in targets {
                adj.entry(source.as_str())
                    .or_default()
                    .push(target.as_str());
                *in_degree.entry(target.as_str()).or_insert(0) += 1;
            }
        }

        // Kahn's algorithm
        let mut queue: VecDeque<&str> = VecDeque::new();
        for (node, degree) in &in_degree {
            if *degree == 0 {
                queue.push_back(node);
            }
        }

        let mut visited_count = 0;
        while let Some(node) = queue.pop_front() {
            visited_count += 1;
            if let Some(neighbors) = adj.get(node) {
                for neighbor in neighbors {
                    if let Some(degree) = in_degree.get_mut(neighbor) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(neighbor);
                        }
                    }
                }
            }
        }

        if visited_count < in_degree.len() {
            // Find the cycle participants
            let cycle_fields: Vec<String> = in_degree
                .iter()
                .filter(|(_, degree)| **degree > 0)
                .map(|(field, _)| (*field).to_string())
                .collect();
            return Err(DagError::CircularDependency {
                fields: cycle_fields,
            });
        }

        Ok(())
    }

    /// Get all fields affected by a change to the given field.
    ///
    /// Returns the transitive closure: if A -> B -> C, changing A returns [B, C].
    /// Results are in topological order (dependencies first).
    pub fn affected_by(&self, changed_field: &str) -> Vec<String> {
        let mut affected = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();

        // Seed with direct dependents
        if let Some(direct) = self.dependents.get(changed_field) {
            for dep in direct {
                if visited.insert(dep.clone()) {
                    queue.push_back(dep.clone());
                }
            }
        }

        // BFS for transitive dependents
        while let Some(field) = queue.pop_front() {
            affected.push(field.clone());
            if let Some(deps) = self.dependents.get(&field) {
                for dep in deps {
                    if visited.insert(dep.clone()) {
                        queue.push_back(dep.clone());
                    }
                }
            }
        }

        affected
    }

    /// Get the topological evaluation order for all computed fields.
    ///
    /// Fields with no dependencies come first.
    pub fn evaluation_order(&self, computed_fields: &[String]) -> Vec<String> {
        let computed_set: HashSet<&String> = computed_fields.iter().collect();

        // Build sub-graph for computed fields only
        let mut in_degree: HashMap<&String, usize> = HashMap::new();
        for field in computed_fields {
            in_degree.insert(field, 0);
        }

        for (source, targets) in &self.dependents {
            for target in targets {
                if computed_set.contains(target) && computed_set.contains(source) {
                    *in_degree.entry(target).or_insert(0) += 1;
                }
            }
        }

        let mut queue: VecDeque<&String> = VecDeque::new();
        for (field, degree) in &in_degree {
            if *degree == 0 {
                queue.push_back(field);
            }
        }

        let mut order = Vec::new();
        while let Some(field) = queue.pop_front() {
            order.push(field.clone());
            if let Some(targets) = self.dependents.get(field.as_str()) {
                for target in targets {
                    if let Some(degree) = in_degree.get_mut(target) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(target);
                        }
                    }
                }
            }
        }

        order
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definition::{FormItem, ItemType};
    use crate::expression::{BinaryOperator, DataType, Expression, FieldProperty};

    fn text_field(link_id: &str) -> FormItem {
        FormItem {
            link_id: link_id.to_string(),
            item_type: ItemType::String,
            text: link_id.to_string(),
            hint: None,
            required: false,
            read_only: false,
            enable_when: None,
            computed_value: None,
            validation: vec![],
            items: vec![],
        }
    }

    #[test]
    fn build_simple_dag() {
        let items = vec![
            text_field("hiv_status"),
            FormItem {
                link_id: "arv_regimen".to_string(),
                item_type: ItemType::String,
                text: "ARV Regimen".to_string(),
                hint: None,
                required: false,
                read_only: false,
                enable_when: Some(Expression::Op {
                    op: BinaryOperator::Eq,
                    left: Box::new(Expression::Field {
                        link_id: "hiv_status".to_string(),
                        property: FieldProperty::Value,
                    }),
                    right: Box::new(Expression::Literal {
                        value: serde_json::json!("positive"),
                        data_type: DataType::String,
                    }),
                }),
                computed_value: None,
                validation: vec![],
                items: vec![],
            },
        ];

        let dag = DependencyDag::build(&items).expect("should build DAG");
        let affected = dag.affected_by("hiv_status");
        assert_eq!(affected, vec!["arv_regimen".to_string()]);
    }

    #[test]
    fn transitive_dependencies() {
        // A -> B -> C
        let items = vec![
            text_field("a"),
            FormItem {
                link_id: "b".to_string(),
                item_type: ItemType::Decimal,
                text: "B".to_string(),
                hint: None,
                required: false,
                read_only: true,
                enable_when: None,
                computed_value: Some(Expression::Field {
                    link_id: "a".to_string(),
                    property: FieldProperty::Value,
                }),
                validation: vec![],
                items: vec![],
            },
            FormItem {
                link_id: "c".to_string(),
                item_type: ItemType::Decimal,
                text: "C".to_string(),
                hint: None,
                required: false,
                read_only: true,
                enable_when: None,
                computed_value: Some(Expression::Field {
                    link_id: "b".to_string(),
                    property: FieldProperty::Value,
                }),
                validation: vec![],
                items: vec![],
            },
        ];

        let dag = DependencyDag::build(&items).expect("should build DAG");
        let affected = dag.affected_by("a");
        assert!(affected.contains(&"b".to_string()));
        assert!(affected.contains(&"c".to_string()));
    }

    #[test]
    fn circular_dependency_detected() {
        // A depends on B, B depends on A
        let items = vec![
            FormItem {
                link_id: "a".to_string(),
                item_type: ItemType::Decimal,
                text: "A".to_string(),
                hint: None,
                required: false,
                read_only: true,
                enable_when: None,
                computed_value: Some(Expression::Field {
                    link_id: "b".to_string(),
                    property: FieldProperty::Value,
                }),
                validation: vec![],
                items: vec![],
            },
            FormItem {
                link_id: "b".to_string(),
                item_type: ItemType::Decimal,
                text: "B".to_string(),
                hint: None,
                required: false,
                read_only: true,
                enable_when: None,
                computed_value: Some(Expression::Field {
                    link_id: "a".to_string(),
                    property: FieldProperty::Value,
                }),
                validation: vec![],
                items: vec![],
            },
        ];

        let result = DependencyDag::build(&items);
        assert!(result.is_err());
        if let Err(DagError::CircularDependency { fields }) = result {
            assert!(fields.contains(&"a".to_string()));
            assert!(fields.contains(&"b".to_string()));
        }
    }

    #[test]
    fn no_dependencies() {
        let items = vec![text_field("a"), text_field("b"), text_field("c")];
        let dag = DependencyDag::build(&items).expect("should build DAG");
        let affected = dag.affected_by("a");
        assert!(affected.is_empty());
    }
}
