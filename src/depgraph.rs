// src/depgraph.rs - Dependency Graph Engine with Full Tracing
use crate::db::Database;
// use crate::model::Package;
use crate::model::InstallReason;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

/// Represents the full dependency graph for installed packages
pub struct DependencyGraph {
    db: Arc<Database>,
    /// Forward dependencies: package -> packages it depends on
    forward: HashMap<String, HashSet<String>>,
    /// Reverse dependencies: package -> packages that depend on it
    reverse: HashMap<String, HashSet<String>>,
}

impl DependencyGraph {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            forward: HashMap::new(),
            reverse: HashMap::new(),
        }
    }

    /// Build the dependency graph from database
    pub fn build(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.forward.clear();
        self.reverse.clear();

        let packages = self.db.get_installed_packages()?;

        for pkg in &packages {
            let deps = self.db.get_dependencies(&pkg.identity.id)?;
            
            self.forward.insert(pkg.identity.id.clone(), deps.iter().cloned().collect());
            
            for dep_id in deps {
                self.reverse
                    .entry(dep_id)
                    .or_insert_with(HashSet::new)
                    .insert(pkg.identity.id.clone());
            }
        }

        Ok(())
    }

    /// Add a dependency relationship
    pub fn add_dependency(&mut self, package_id: &str, dependency_id: &str) {
        self.forward
            .entry(package_id.to_string())
            .or_insert_with(HashSet::new)
            .insert(dependency_id.to_string());

        self.reverse
            .entry(dependency_id.to_string())
            .or_insert_with(HashSet::new)
            .insert(package_id.to_string());

        // Also persist to database
        let _ = self.db.add_dependency(package_id, dependency_id, "runtime");
    }

    /// Remove a package from the graph
    pub fn remove_package(&mut self, package_id: &str) {
        // Remove from forward deps
        self.forward.remove(package_id);

        // Remove from all reverse deps
        for deps in self.reverse.values_mut() {
            deps.remove(package_id);
        }

        // Remove from forward deps of other packages
        for deps in self.forward.values_mut() {
            deps.remove(package_id);
        }

        // Clean up reverse entry
        self.reverse.remove(package_id);
    }

    /// Get all direct dependencies of a package
    pub fn get_dependencies(&self, package_id: &str) -> Vec<String> {
        self.forward
            .get(package_id)
            .map(|deps| deps.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get all packages that directly depend on this package
    pub fn get_reverse_dependencies(&self, package_id: &str) -> Vec<String> {
        self.reverse
            .get(package_id)
            .map(|deps| deps.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get the full dependency tree (all transitive dependencies)
    pub fn get_full_dependency_tree(&self, package_id: &str) -> DependencyTree {
        let mut visited = HashSet::new();
        let mut tree = DependencyTree::new(package_id.to_string());
        
        self.build_tree_recursive(package_id, &mut tree.root, &mut visited);
        
        tree
    }

    fn build_tree_recursive(
        &self,
        package_id: &str,
        node: &mut TreeNode,
        visited: &mut HashSet<String>,
    ) {
        if visited.contains(package_id) {
            node.is_circular = true;
            return;
        }
        visited.insert(package_id.to_string());

        if let Some(deps) = self.forward.get(package_id) {
            for dep_id in deps {
                let mut child = TreeNode::new(dep_id.clone());
                self.build_tree_recursive(dep_id, &mut child, visited);
                node.children.push(child);
            }
        }

        visited.remove(package_id);
    }

    /// Trace the path from an app to a specific dependency
    pub fn trace_dependency(&self, app_id: &str, dep_id: &str) -> Option<DependencyTrace> {
        let mut visited = HashSet::new();
        let mut path = Vec::new();
        
        if self.trace_path(app_id, dep_id, &mut visited, &mut path) {
            Some(DependencyTrace {
                app_id: app_id.to_string(),
                dependency_id: dep_id.to_string(),
                path,
            })
        } else {
            None
        }
    }

    fn trace_path(
        &self,
        current: &str,
        target: &str,
        visited: &mut HashSet<String>,
        path: &mut Vec<String>,
    ) -> bool {
        if current == target {
            path.push(current.to_string());
            return true;
        }

        if visited.contains(current) {
            return false;
        }
        visited.insert(current.to_string());
        path.push(current.to_string());

        if let Some(deps) = self.forward.get(current) {
            for dep in deps {
                if self.trace_path(dep, target, visited, path) {
                    return true;
                }
            }
        }

        path.pop();
        false
    }

    /// Find orphaned dependencies (installed as deps but no longer needed)
    pub fn find_orphans(&self) -> Vec<String> {
        let mut orphans = Vec::new();
        
        for (pkg_id, reverse_deps) in &self.reverse {
            // A package is an orphan if:
            // 1. It has no reverse dependencies (nothing depends on it)
            // 2. It was installed as a dependency (not explicitly)
            if reverse_deps.is_empty() {
                // Check if it was installed as a dependency
                if let Ok(Some(pkg)) = self.db.get_package(pkg_id) {
                    if matches!(pkg.dependency_info.install_reason, InstallReason::Dependency(_)) {
                        orphans.push(pkg_id.clone());
                    }
                }
            }
        }
        
        orphans
    }

    /// Check for circular dependencies
    pub fn detect_cycles(&self) -> Vec<Vec<String>> {
        let mut cycles = Vec::new();
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();
        let mut path = Vec::new();

        for pkg_id in self.forward.keys() {
            if !visited.contains(pkg_id) {
                self.detect_cycle_dfs(pkg_id, &mut visited, &mut rec_stack, &mut path, &mut cycles);
            }
        }

        cycles
    }

    fn detect_cycle_dfs(
        &self,
        current: &str,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
        path: &mut Vec<String>,
        cycles: &mut Vec<Vec<String>>,
    ) {
        visited.insert(current.to_string());
        rec_stack.insert(current.to_string());
        path.push(current.to_string());

        if let Some(deps) = self.forward.get(current) {
            for dep in deps {
                if !visited.contains(dep) {
                    self.detect_cycle_dfs(dep, visited, rec_stack, path, cycles);
                } else if rec_stack.contains(dep) {
                    // Found a cycle
                    let cycle_start = path.iter().position(|p| p == dep).unwrap();
                    let cycle: Vec<String> = path[cycle_start..].to_vec();
                    cycles.push(cycle);
                }
            }
        }

        path.pop();
        rec_stack.remove(current);
    }

    /// Perform topological sort (for installation order)
    pub fn topological_sort(&self, package_ids: &[String]) -> Result<Vec<String>, String> {
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut subset_forward: HashMap<String, HashSet<String>> = HashMap::new();

        // Build subset of graph
        let package_set: HashSet<_> = package_ids.iter().cloned().collect();
        
        for pkg_id in package_ids {
            in_degree.entry(pkg_id.clone()).or_insert(0);
            
            if let Some(deps) = self.forward.get(pkg_id) {
                for dep in deps {
                    if package_set.contains(dep) {
                        subset_forward
                            .entry(pkg_id.clone())
                            .or_insert_with(HashSet::new)
                            .insert(dep.clone());
                        *in_degree.entry(dep.clone()).or_insert(0) += 1;
                    }
                }
            }
        }

        // Kahn's algorithm
        let mut queue: VecDeque<String> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(id, _)| id.clone())
            .collect();

        let mut result = Vec::new();

        while let Some(pkg) = queue.pop_front() {
            result.push(pkg.clone());

            if let Some(deps) = subset_forward.get(&pkg) {
                for dep in deps {
                    if let Some(deg) = in_degree.get_mut(dep) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(dep.clone());
                        }
                    }
                }
            }
        }

        if result.len() != package_ids.len() {
            return Err("Circular dependency detected".to_string());
        }

        // Reverse because we added packages with no deps first (those should be installed first)
        result.reverse();
        Ok(result)
    }

    /// Get dependency statistics
    pub fn get_stats(&self) -> DependencyStats {
        let total_packages = self.forward.len();
        let total_dependencies: usize = self.forward.values().map(|d| d.len()).sum();
        let max_depth = self.calculate_max_depth();
        let orphan_count = self.find_orphans().len();

        DependencyStats {
            total_packages,
            total_dependencies,
            average_dependencies: if total_packages > 0 {
                total_dependencies as f32 / total_packages as f32
            } else {
                0.0
            },
            max_depth,
            orphan_count,
        }
    }

    fn calculate_max_depth(&self) -> usize {
        let mut max_depth = 0;
        
        for pkg_id in self.forward.keys() {
            let depth = self.get_depth(pkg_id, &mut HashSet::new());
            max_depth = max_depth.max(depth);
        }
        
        max_depth
    }

    fn get_depth(&self, package_id: &str, visited: &mut HashSet<String>) -> usize {
        if visited.contains(package_id) {
            return 0; // Cycle, don't count
        }
        visited.insert(package_id.to_string());

        let max_child_depth = self
            .forward
            .get(package_id)
            .map(|deps| {
                deps.iter()
                    .map(|d| self.get_depth(d, visited))
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);

        visited.remove(package_id);
        1 + max_child_depth
    }
}

/// Represents the full dependency tree for a package
#[derive(Debug, Clone)]
pub struct DependencyTree {
    pub package_id: String,
    pub root: TreeNode,
}

impl DependencyTree {
    fn new(package_id: String) -> Self {
        Self {
            package_id: package_id.clone(),
            root: TreeNode::new(package_id),
        }
    }

    /// Flatten the tree to a list of all dependencies
    pub fn flatten(&self) -> Vec<String> {
        let mut result = Vec::new();
        self.flatten_recursive(&self.root, &mut result);
        result
    }

    fn flatten_recursive(&self, node: &TreeNode, result: &mut Vec<String>) {
        for child in &node.children {
            if !result.contains(&child.package_id) {
                result.push(child.package_id.clone());
                self.flatten_recursive(child, result);
            }
        }
    }

    /// Get the depth of the dependency tree
    pub fn depth(&self) -> usize {
        self.node_depth(&self.root)
    }

    fn node_depth(&self, node: &TreeNode) -> usize {
        if node.children.is_empty() {
            0
        } else {
            1 + node.children.iter().map(|c| self.node_depth(c)).max().unwrap_or(0)
        }
    }
}

#[derive(Debug, Clone)]
pub struct TreeNode {
    pub package_id: String,
    pub children: Vec<TreeNode>,
    pub is_circular: bool,
}

impl TreeNode {
    fn new(package_id: String) -> Self {
        Self {
            package_id,
            children: Vec::new(),
            is_circular: false,
        }
    }
}

/// Trace showing the path from an app to a dependency
#[derive(Debug, Clone)]
pub struct DependencyTrace {
    pub app_id: String,
    pub dependency_id: String,
    pub path: Vec<String>,
}

impl DependencyTrace {
    pub fn display(&self) -> String {
        self.path.join(" → ")
    }
}

/// Statistics about the dependency graph
#[derive(Debug, Clone)]
pub struct DependencyStats {
    pub total_packages: usize,
    pub total_dependencies: usize,
    pub average_dependencies: f32,
    pub max_depth: usize,
    pub orphan_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn create_test_graph() -> DependencyGraph {
        let db = Arc::new(Database::open(&PathBuf::from(":memory:")).unwrap());
        let mut graph = DependencyGraph::new(db);
        
        // Create a simple dependency chain: app -> lib1 -> lib2
        graph.add_dependency("app", "lib1");
        graph.add_dependency("lib1", "lib2");
        graph.add_dependency("app", "lib3");
        
        graph
    }

    #[test]
    fn test_dependency_lookup() {
        let graph = create_test_graph();
        
        let deps = graph.get_dependencies("app");
        assert!(deps.contains(&"lib1".to_string()));
        assert!(deps.contains(&"lib3".to_string()));
    }

    #[test]
    fn test_reverse_dependencies() {
        let graph = create_test_graph();
        
        let reverse = graph.get_reverse_dependencies("lib1");
        assert!(reverse.contains(&"app".to_string()));
    }

    #[test]
    fn test_trace_dependency() {
        let graph = create_test_graph();
        
        let trace = graph.trace_dependency("app", "lib2");
        assert!(trace.is_some());
        
        let trace = trace.unwrap();
        assert_eq!(trace.path, vec!["app", "lib1", "lib2"]);
    }

    #[test]
    fn test_topological_sort() {
        let graph = create_test_graph();
        
        let packages = vec!["app".to_string(), "lib1".to_string(), "lib2".to_string()];
        let sorted = graph.topological_sort(&packages).unwrap();
        
        // lib2 should come before lib1, and lib1 should come before app
        let lib2_pos = sorted.iter().position(|p| p == "lib2").unwrap();
        let lib1_pos = sorted.iter().position(|p| p == "lib1").unwrap();
        let app_pos = sorted.iter().position(|p| p == "app").unwrap();
        
        assert!(lib2_pos < lib1_pos);
        assert!(lib1_pos < app_pos);
    }
}
