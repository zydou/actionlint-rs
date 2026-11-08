//! The workflow syntax tree visitor.
//!
//! Rust counterpart of the original `pass.go`. A [`Visitor`] drives a set of [`Pass`] values
//! through a [`Workflow`] tree in depth-first order.

use std::io::Write;
use std::time::Instant;

use crate::ast::{Job, Step, Workflow};
use crate::rule::Pass;

/// Visits a syntax tree from the root in depth-first order, calling every registered [`Pass`].
///
/// The visitor borrows the passes rather than owning them so that callers retain access to the
/// passes (e.g. to collect the errors a [`crate::rule::Rule`] produced) after traversal.
#[derive(Default)]
pub struct Visitor<'a> {
    passes: Vec<&'a mut dyn Pass>,
    debug: Option<Box<dyn Write>>,
}

impl<'a> Visitor<'a> {
    /// Creates a new `Visitor`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a pass that is called while traversing the syntax tree.
    pub fn add_pass(&mut self, pass: &'a mut dyn Pass) {
        self.passes.push(pass);
    }

    /// Adds a [`Rule`](crate::rule::Rule) as a pass. The rule is upcast to a [`Pass`] trait
    /// object so it participates in the traversal.
    pub fn add_pass_box(&mut self, pass: &'a mut dyn crate::rule::Rule) {
        self.passes.push(pass);
    }

    /// Enables debug output to the given writer. All debug output from the visitor is written
    /// there.
    pub fn enable_debug(&mut self, out: Box<dyn Write>) {
        self.debug = Some(out);
    }

    fn report_elapsed(&mut self, what: &str, start: Instant) {
        if let Some(w) = &mut self.debug {
            let _ = writeln!(w, "[Visitor] {what} took {}ms", start.elapsed().as_millis());
        }
    }

    /// Visits the given syntax tree in depth-first order.
    pub fn visit(&mut self, workflow: &Workflow) -> Result<(), String> {
        let start = Instant::now();
        for p in &mut self.passes {
            p.visit_workflow_pre(workflow)?;
        }
        self.report_elapsed("VisitWorkflowPre", start);

        let start = Instant::now();
        for job in workflow.jobs.values() {
            self.visit_job(job)?;
        }
        self.report_elapsed(&format!("Visiting {} jobs", workflow.jobs.len()), start);

        let start = Instant::now();
        for p in &mut self.passes {
            p.visit_workflow_post(workflow)?;
        }
        self.report_elapsed("VisitWorkflowPost", start);

        Ok(())
    }

    fn visit_job(&mut self, job: &Job) -> Result<(), String> {
        let start = Instant::now();
        for p in &mut self.passes {
            p.visit_job_pre(job)?;
        }
        self.report_elapsed(&format!("VisitWorkflowJobPre at job {:?}", job.id), start);

        let start = Instant::now();
        for step in &job.steps {
            self.visit_step(step)?;
        }
        self.report_elapsed(
            &format!("Visiting {} steps at job {:?}", job.steps.len(), job.id),
            start,
        );

        let start = Instant::now();
        for p in &mut self.passes {
            p.visit_job_post(job)?;
        }
        self.report_elapsed(&format!("VisitWorkflowJobPost at job {:?}", job.id), start);

        Ok(())
    }

    fn visit_step(&mut self, step: &Step) -> Result<(), String> {
        let start = Instant::now();
        for p in &mut self.passes {
            p.visit_step(step)?;
        }
        self.report_elapsed(&format!("VisitStep at {}", step.pos), start);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Exec, ExecRun, Pos};
    use std::sync::{Arc, Mutex};

    struct RecordingPass {
        log: Arc<Mutex<Vec<String>>>,
    }

    impl Pass for RecordingPass {
        fn visit_workflow_pre(&mut self, _n: &Workflow) -> Result<(), String> {
            self.log.lock().unwrap().push("wf-pre".into());
            Ok(())
        }

        fn visit_workflow_post(&mut self, _n: &Workflow) -> Result<(), String> {
            self.log.lock().unwrap().push("wf-post".into());
            Ok(())
        }

        fn visit_job_pre(&mut self, n: &Job) -> Result<(), String> {
            self.log.lock().unwrap().push(format!("job-pre {}", n.id));
            Ok(())
        }

        fn visit_job_post(&mut self, n: &Job) -> Result<(), String> {
            self.log.lock().unwrap().push(format!("job-post {}", n.id));
            Ok(())
        }

        fn visit_step(&mut self, n: &Step) -> Result<(), String> {
            self.log.lock().unwrap().push(format!("step {}", n.pos));
            Ok(())
        }
    }

    fn sample() -> Workflow {
        let step = Step {
            id: None,
            if_condition: None,
            name: None,
            exec: Exec::Run(ExecRun {
                run: "echo".into(),
                shell: None,
                working_directory: None,
                run_pos: Pos::new(6, 9),
            }),
            env: None,
            continue_on_error: None,
            timeout_minutes: None,
            pos: Pos::new(6, 7),
        };
        let job = Job {
            id: "test".into(),
            name: None,
            needs: vec![],
            runs_on: None,
            permissions: None,
            environment: None,
            concurrency: None,
            outputs: Default::default(),
            env: None,
            defaults: None,
            if_condition: None,
            steps: vec![step],
            timeout_minutes: None,
            strategy: None,
            continue_on_error: None,
            container: None,
            services: None,
            workflow_call: None,
            snapshot: None,
            pos: Pos::new(3, 3),
        };
        let mut jobs = std::collections::BTreeMap::new();
        jobs.insert("test".to_string(), job);
        Workflow {
            name: None,
            run_name: None,
            on: vec![],
            permissions: None,
            env: None,
            defaults: None,
            concurrency: None,
            jobs,
        }
    }

    #[test]
    fn visit_order() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut pass = RecordingPass { log: log.clone() };
        let mut v = Visitor::new();
        v.add_pass(&mut pass);
        v.visit(&sample()).unwrap();

        let log = log.lock().unwrap().clone();
        assert_eq!(
            log,
            vec![
                "wf-pre",
                "job-pre test",
                "step line:6,col:7",
                "job-post test",
                "wf-post",
            ]
        );
    }

    #[test]
    fn error_propagates() {
        struct Fail;
        impl Pass for Fail {
            fn visit_job_pre(&mut self, _n: &Job) -> Result<(), String> {
                Err("boom".into())
            }
        }
        let mut fail = Fail;
        let mut v = Visitor::new();
        v.add_pass(&mut fail);
        let err = v.visit(&sample()).unwrap_err();
        assert_eq!(err, "boom");
    }
}
