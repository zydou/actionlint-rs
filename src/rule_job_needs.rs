//! The `job-needs` rule.
//!
//! Checks: Checks for job IDs in "needs:". Undefined IDs and cyclic dependencies are checked.
//! Rust counterpart of the original `rule_job_needs.go`. See
//! <https://docs.github.com/en/actions/learn-github-actions/workflow-syntax-for-github-actions#jobsjob_idneeds>.

use std::collections::HashMap;

use crate::ast::{Job, Pos, Workflow};
use crate::rule::{Pass, Rule, RuleBase};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeStatus {
    New,
    Active,
    Finished,
}

/// A node in the "needs" dependency graph. Nodes are stored in a [`Vec`] and referenced by index,
/// mirroring the pointer-identity graph in the original Go implementation.
#[derive(Debug)]
struct JobNode {
    id: std::string::String,
    /// Lowercased "needs" values as declared by the user.
    needs: Vec<std::string::String>,
    /// Indices (into the owning `Vec`) of the resolved dependencies.
    resolved: Vec<usize>,
    status: NodeStatus,
    pos: Pos,
}

/// The `job-needs` rule.
#[derive(Debug)]
pub struct RuleJobNeeds {
    base: RuleBase,
    /// Nodes in declaration order.
    nodes: Vec<JobNode>,
    /// Lowercased job ID -> index into [`RuleJobNeeds::nodes`].
    by_id: HashMap<std::string::String, usize>,
}

impl RuleJobNeeds {
    /// Creates a new `job-needs` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new(
                "job-needs",
                "Checks for job IDs in \"needs:\". Undefined IDs and cyclic dependencies are checked",
            ),
            nodes: Vec::new(),
            by_id: HashMap::new(),
        }
    }
}

impl Default for RuleJobNeeds {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RuleJobNeeds {
    fn visit_job_pre(&mut self, n: &Job) -> Result<(), String> {
        let mut needs: Vec<std::string::String> = Vec::with_capacity(n.needs.len());
        for j in &n.needs {
            let id = j.value.to_lowercase();
            if needs.contains(&id) {
                self.base.error_at(
                    j.pos,
                    format!(
                        "job ID {:?} duplicates in \"needs\" section. note that job ID is case insensitive",
                        j.value
                    ),
                );
                continue;
            }
            if !id.is_empty() {
                // Job ID is the key of a mapping. Keys are stored lowercase since job IDs are case
                // insensitive, so values in the 'needs' array must be compared lowercase.
                needs.push(id);
            }
        }

        let id = n.id.value.to_lowercase();
        if id.is_empty() {
            return Ok(());
        }
        if let Some(&prev) = self.by_id.get(&id) {
            self.base.error_at(
                n.id.pos,
                format!(
                    "job ID {:?} duplicates. previously defined at {}. note that job ID is case insensitive",
                    n.id.value, self.nodes[prev].pos
                ),
            );
        }

        let idx = self.nodes.len();
        self.nodes.push(JobNode {
            id: id.clone(),
            needs,
            resolved: Vec::new(),
            status: NodeStatus::New,
            pos: n.id.pos,
        });
        self.by_id.insert(id, idx);

        Ok(())
    }

    fn visit_workflow_post(&mut self, _n: &Workflow) -> Result<(), String> {
        use std::fmt::Write as _;

        // Resolve nodes. Node lookups by index are done against the node list captured here.
        let mut valid = true;
        for i in 0..self.nodes.len() {
            let needs = std::mem::take(&mut self.nodes[i].needs);
            let mut resolved = Vec::with_capacity(needs.len());
            for dep in needs {
                match self.by_id.get(&dep) {
                    None => {
                        self.base.error_at(
                            self.nodes[i].pos,
                            format!(
                                "job {:?} needs job {:?} which does not exist in this workflow",
                                self.nodes[i].id, dep
                            ),
                        );
                        valid = false;
                    }
                    Some(&j) => {
                        resolved.push(j);
                    }
                }
            }
            self.nodes[i].resolved = resolved;
        }
        if !valid {
            return Ok(());
        }

        // Note: Only the first cycle can be detected even if there are multiple cycles.
        let Some((from, to)) = detect_first_cycle(&mut self.nodes) else {
            return Ok(());
        };

        // Collect the full cycle starting from the first detected back edge.
        let mut edges: HashMap<usize, usize> = HashMap::new();
        edges.insert(from, to);
        collect_cycle(&mut self.nodes, to, &mut edges);

        // Start the cycle from the smallest position for a deterministic error message.
        let mut start = from;
        for &n in edges.keys() {
            if self.nodes[n].pos.is_before(&self.nodes[start].pos) {
                start = n;
            }
        }

        let mut msg = String::new();
        msg.push_str(
            "cyclic dependencies in \"needs\" job configurations are detected. detected cycle is ",
        );

        let _ = write!(msg, "{:?}", self.nodes[start].id);
        let mut cur = start;
        loop {
            let next = edges[&cur];
            let _ = write!(msg, " -> {:?}", self.nodes[next].id);
            cur = next;
            if cur == start {
                break;
            }
        }

        self.base.error_at(self.nodes[start].pos, msg);

        Ok(())
    }
}

impl Rule for RuleJobNeeds {
    fn take_errors(&mut self) -> Vec<crate::error::Error> {
        self.base.take_errors()
    }

    fn name(&self) -> &'static str {
        self.base.name()
    }

    fn description(&self) -> &'static str {
        self.base.description()
    }

    fn enable_debug(&mut self, out: Box<dyn std::io::Write + Send>) {
        self.base.enable_debug(out);
    }

    fn set_config(&mut self, config: crate::config::Config) {
        self.base.set_config(config);
    }
}

/// Collects the cycle starting from `src`, following only active nodes. Counterpart of `collectCycle`.
fn collect_cycle(nodes: &mut [JobNode], src: usize, edges: &mut HashMap<usize, usize>) -> bool {
    for dest in nodes[src].resolved.clone() {
        if nodes[dest].status != NodeStatus::Active {
            continue;
        }
        edges.insert(src, dest);
        if edges.contains_key(&dest) {
            return true;
        }
        if collect_cycle(nodes, dest, edges) {
            return true;
        }
        edges.remove(&src);
    }
    false
}

/// Detects the first cyclic dependency. Counterpart of `detectFirstCycle`.
fn detect_first_cycle(nodes: &mut [JobNode]) -> Option<(usize, usize)> {
    for i in 0..nodes.len() {
        if nodes[i].status == NodeStatus::New
            && let Some(e) = detect_cyclic_node(nodes, i)
        {
            return Some(e);
        }
    }
    None
}

/// DFS helper detecting a cyclic dependency. Counterpart of `detectCyclicNode`.
fn detect_cyclic_node(nodes: &mut [JobNode], v: usize) -> Option<(usize, usize)> {
    nodes[v].status = NodeStatus::Active;
    for &w in &nodes[v].resolved.clone() {
        match nodes[w].status {
            NodeStatus::Active => return Some((v, w)),
            NodeStatus::New => {
                if let Some(e) = detect_cyclic_node(nodes, w) {
                    return Some(e);
                }
            }
            NodeStatus::Finished => {}
        }
    }
    nodes[v].status = NodeStatus::Finished;
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Exec, ExecRun, Step, String};

    fn job(id: &str, needs: Vec<&str>) -> Job {
        Job {
            id: String::at(id, Pos::new(3, 3)),
            name: None,
            needs: needs
                .into_iter()
                .map(|n| String::at(n, Pos::new(4, 7)))
                .collect(),
            runs_on: None,
            permissions: None,
            environment: None,
            concurrency: None,
            outputs: Default::default(),
            env: None,
            defaults: None,
            if_condition: None,
            steps: vec![Step {
                id: None,
                if_condition: None,
                name: None,
                exec: Exec::Run(ExecRun {
                    run: String::at("echo", Pos::new(6, 9)),
                    shell: None,
                    working_directory: None,
                    run_pos: Pos::new(6, 9),
                }),
                env: None,
                continue_on_error: None,
                timeout_minutes: None,
                pos: Pos::new(6, 7),
            }],
            timeout_minutes: None,
            strategy: None,
            continue_on_error: None,
            container: None,
            services: None,
            workflow_call: None,
            snapshot: None,
            pos: Pos::new(3, 3),
        }
    }

    fn wf_with_jobs(jobs: Vec<Job>) -> Workflow {
        let mut m = std::collections::BTreeMap::new();
        for j in jobs {
            m.insert(j.id.value.to_lowercase(), j);
        }
        Workflow {
            name: None,
            run_name: None,
            on: vec![],
            permissions: None,
            env: None,
            defaults: None,
            concurrency: None,
            jobs: m,
        }
    }

    #[test]
    fn undefined_need() {
        let mut r = RuleJobNeeds::new();
        let wf = wf_with_jobs(vec![job("a", vec!["nonexistent"])]);
        r.visit_job_pre(&wf.jobs["a"]).unwrap();
        r.visit_workflow_post(&wf).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0]
                .message
                .contains("which does not exist in this workflow")
        );
    }

    #[test]
    fn duplicate_need() {
        let mut r = RuleJobNeeds::new();
        let mut j = job("a", vec![]);
        j.needs = vec![
            String::at("b", Pos::new(4, 7)),
            String::at("B", Pos::new(4, 9)),
        ];
        r.visit_job_pre(&j).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("duplicates in \"needs\" section"));
    }

    #[test]
    fn duplicate_job_id() {
        let mut r = RuleJobNeeds::new();
        let a1 = job("build", vec![]);
        let mut a2 = job("Build", vec![]);
        // Give it a different position so the message points at the redefinition.
        a2.id = String::at("Build", Pos::new(20, 3));
        r.visit_job_pre(&a1).unwrap();
        r.visit_job_pre(&a2).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0]
                .message
                .contains("duplicates. previously defined at")
        );
        assert!(errs[0].message.contains("case insensitive"));
    }

    #[test]
    fn valid_needs() {
        let mut r = RuleJobNeeds::new();
        let wf = wf_with_jobs(vec![job("a", vec![]), job("b", vec!["a"])]);
        for j in wf.jobs.values() {
            r.visit_job_pre(j).unwrap();
        }
        r.visit_workflow_post(&wf).unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn cyclic_needs() {
        let mut r = RuleJobNeeds::new();
        let wf = wf_with_jobs(vec![
            job("a", vec!["b"]),
            job("b", vec!["c"]),
            job("c", vec!["a"]),
        ]);
        for j in wf.jobs.values() {
            r.visit_job_pre(j).unwrap();
        }
        r.visit_workflow_post(&wf).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0]
                .message
                .contains("cyclic dependencies in \"needs\" job configurations are detected")
        );
        assert!(errs[0].message.contains("-> "));
    }

    #[test]
    fn self_cycle() {
        let mut r = RuleJobNeeds::new();
        let wf = wf_with_jobs(vec![job("a", vec!["a"])]);
        r.visit_job_pre(&wf.jobs["a"]).unwrap();
        r.visit_workflow_post(&wf).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("cyclic dependencies"));
    }
}
