use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};

use crate::ProjectConfig;

/// A validated, acyclic dependency graph of build targets.
#[derive(Debug, Clone)]
pub struct TargetGraph {
    deps: BTreeMap<String, Vec<String>>,
}

impl TargetGraph {
    /// Builds and validates the graph: every dependency must exist and the
    /// graph must not contain cycles.
    pub fn new(config: &ProjectConfig) -> Result<Self> {
        let deps: BTreeMap<_, _> = config
            .targets
            .iter()
            .map(|(name, target)| (name.clone(), target.deps.clone()))
            .collect();

        for (name, target_deps) in &deps {
            for dep in target_deps {
                if !deps.contains_key(dep) {
                    bail!("target `{name}` depends on unknown target `{dep}`");
                }
            }
        }

        let graph = Self { deps };
        graph.topological_order()?;
        Ok(graph)
    }

    /// Direct dependencies of `target`, or `None` if it does not exist.
    pub fn deps(&self, target: &str) -> Option<&[String]> {
        self.deps.get(target).map(Vec::as_slice)
    }

    /// All target names, in deterministic order.
    pub fn targets(&self) -> impl Iterator<Item = &str> {
        self.deps.keys().map(String::as_str)
    }

    /// Returns targets ordered so that every target comes after its
    /// dependencies. Fails if the graph contains a cycle.
    pub fn topological_order(&self) -> Result<Vec<&str>> {
        #[derive(Clone, Copy, PartialEq)]
        enum Mark {
            Visiting,
            Done,
        }

        fn visit<'a>(
            graph: &'a TargetGraph,
            name: &'a str,
            marks: &mut BTreeMap<&'a str, Mark>,
            stack: &mut Vec<&'a str>,
            order: &mut Vec<&'a str>,
        ) -> Result<()> {
            match marks.get(name) {
                Some(Mark::Done) => return Ok(()),
                Some(Mark::Visiting) => {
                    let start = stack.iter().position(|n| *n == name).unwrap_or(0);
                    let mut cycle = stack[start..].to_vec();
                    cycle.push(name);
                    bail!("dependency cycle detected: {}", cycle.join(" -> "));
                }
                None => {}
            }
            marks.insert(name, Mark::Visiting);
            stack.push(name);
            for dep in &graph.deps[name] {
                visit(graph, dep, marks, stack, order)?;
            }
            stack.pop();
            marks.insert(name, Mark::Done);
            order.push(name);
            Ok(())
        }

        let mut marks = BTreeMap::new();
        let mut stack = Vec::new();
        let mut order = Vec::with_capacity(self.deps.len());
        for name in self.deps.keys() {
            visit(self, name, &mut marks, &mut stack, &mut order)?;
        }
        Ok(order)
    }

    /// `target` plus all of its transitive dependencies.
    pub fn closure(&self, target: &str) -> BTreeSet<&str> {
        let mut seen = BTreeSet::new();
        let mut pending = vec![target];
        while let Some(name) = pending.pop() {
            if let Some((key, deps)) = self.deps.get_key_value(name)
                && seen.insert(key.as_str())
            {
                pending.extend(deps.iter().map(String::as_str));
            }
        }
        seen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(source: &str) -> ProjectConfig {
        ProjectConfig::from_turbo_json(source, None).unwrap()
    }

    #[test]
    fn orders_dependencies_first() {
        let graph = TargetGraph::new(&config(
            r#"{
              "tasks": {
                "app": { "dependsOn": ["lib", "util"] },
                "lib": { "dependsOn": ["util"] },
                "util": {}
              }
            }"#,
        ))
        .unwrap();

        assert_eq!(graph.topological_order().unwrap(), ["util", "lib", "app"]);
        assert_eq!(
            graph.closure("lib").into_iter().collect::<Vec<_>>(),
            ["lib", "util"]
        );
    }

    #[test]
    fn rejects_unknown_dependency() {
        let err = TargetGraph::new(&config(
            r#"{ "tasks": { "a": { "dependsOn": ["missing"] } } }"#,
        ))
        .unwrap_err();
        assert!(err.to_string().contains("unknown target `missing`"));
    }

    #[test]
    fn rejects_cycles() {
        let err = TargetGraph::new(&config(
            r#"{ "tasks": { "a": { "dependsOn": ["b"] }, "b": { "dependsOn": ["a"] } } }"#,
        ))
        .unwrap_err();
        assert!(err.to_string().contains("dependency cycle detected"));
    }
}
