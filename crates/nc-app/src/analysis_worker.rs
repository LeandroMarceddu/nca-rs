//! Background analysis worker.
//!
//! `nc_analysis::analyze` is deterministic but can take seconds on a large
//! program, so it runs on a named thread, never on the UI thread. A monotonic
//! generation makes every superseded request inert: queued jobs are drained to
//! the newest one, and a result whose generation is no longer current is
//! discarded by the shell before it can replace a newer file's report.
//!
//! Cancellation is cooperative at job boundaries. The analysis engine itself
//! has no mid-run cancel hook, so a request already inside `analyze` finishes
//! its work and its result is dropped. That limitation is reported as an
//! interface gap, not hidden.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crossbeam_channel::{Receiver, Sender};
use nc_analysis::{AnalysisConfig, AnalysisRequest, Report, RuleRegistry, analyze, rules};
use nc_core::Dialect;
use nc_gcode::Program;

use crate::analysis_ui::AnalysisGeneration;

/// One analysis request.
struct AnalysisJob {
    generation: AnalysisGeneration,
    program: Arc<Program>,
    file_name: String,
    source: Arc<str>,
    config: AnalysisConfig,
}

/// One finished analysis. Stale generations are discarded by the shell.
pub struct AnalysisResponse {
    /// Generation this result belongs to.
    pub generation: AnalysisGeneration,
    /// The report, or a plain-language failure.
    pub result: Result<Report, String>,
}

/// Handle to the analysis thread.
pub struct AnalysisWorker {
    jobs: Sender<AnalysisJob>,
    results: Receiver<AnalysisResponse>,
    latest: Arc<AtomicU64>,
    alive: Arc<AtomicBool>,
}

impl AnalysisWorker {
    /// Spawns the worker thread.
    pub fn spawn() -> Result<Self, String> {
        let (job_tx, job_rx) = crossbeam_channel::unbounded::<AnalysisJob>();
        let (result_tx, result_rx) = crossbeam_channel::unbounded::<AnalysisResponse>();
        let latest = Arc::new(AtomicU64::new(0));
        let alive = Arc::new(AtomicBool::new(true));
        let worker_latest = Arc::clone(&latest);
        let worker_alive = Arc::clone(&alive);
        std::thread::Builder::new()
            .name("nca-analysis".to_string())
            .spawn(move || worker_loop(&job_rx, &result_tx, &worker_latest, &worker_alive))
            .map_err(|error| format!("could not start the analysis worker: {error}"))?;
        Ok(Self {
            jobs: job_tx,
            results: result_rx,
            latest,
            alive,
        })
    }

    /// Queues an analysis. Any queued or running older request becomes stale.
    pub fn request(
        &mut self,
        generation: AnalysisGeneration,
        program: Arc<Program>,
        file_name: String,
        source: Arc<str>,
        config: AnalysisConfig,
    ) -> Result<(), String> {
        self.latest.store(generation.run, Ordering::Release);
        self.jobs
            .send(AnalysisJob {
                generation,
                program,
                file_name,
                source,
                config,
            })
            .map_err(|_| "the analysis worker stopped unexpectedly".to_string())
    }

    /// Marks every queued or running result stale without starting a new run.
    pub fn cancel(&mut self, next_run: u64) {
        self.latest.store(next_run, Ordering::Release);
    }

    /// Non-blocking receive of one finished analysis.
    #[must_use]
    pub fn try_recv(&self) -> Option<AnalysisResponse> {
        self.results.try_recv().ok()
    }

    /// `false` after the worker thread exits.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }
}

fn worker_loop(
    jobs: &Receiver<AnalysisJob>,
    results: &Sender<AnalysisResponse>,
    latest: &AtomicU64,
    alive: &AtomicBool,
) {
    while let Ok(mut job) = jobs.recv() {
        while let Ok(newer) = jobs.try_recv() {
            job = newer;
        }
        if job.generation.run != latest.load(Ordering::Acquire) {
            continue;
        }
        let registry = RuleRegistry::from_rules(rules::all());
        let request =
            AnalysisRequest::new(&job.file_name, &job.source, &job.config, Dialect::Generic)
                .with_dialect_assumed(true);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            analyze(&job.program, &request, &registry)
        }))
        .map_err(|_| "analysis panicked on this input; no report was produced".to_string());
        if job.generation.run != latest.load(Ordering::Acquire) {
            continue;
        }
        if results
            .send(AnalysisResponse {
                generation: job.generation,
                result,
            })
            .is_err()
        {
            alive.store(false, Ordering::Release);
            return;
        }
    }
    alive.store(false, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn worker_delivers_only_the_latest_generation() {
        let source = "G21 G90\nG1 X10 F100\nG1 Y10\n";
        let program = Arc::new(nc_gcode::parse_program(source).expect("program"));
        let config = AnalysisConfig::default();
        let mut worker = AnalysisWorker::spawn().expect("worker");
        let stale = AnalysisGeneration { run: 1, program: 1 };
        let current = AnalysisGeneration { run: 2, program: 1 };
        worker
            .request(
                stale,
                Arc::clone(&program),
                "part.nc".to_string(),
                Arc::from(source),
                config.clone(),
            )
            .expect("stale request");
        worker
            .request(
                current,
                program,
                "part.nc".to_string(),
                Arc::from(source),
                config,
            )
            .expect("current request");

        let mut accepted = None;
        for _ in 0..300 {
            while let Some(response) = worker.try_recv() {
                if response.generation == current {
                    accepted = Some(response);
                    break;
                }
            }
            if accepted.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let response = accepted.expect("latest analysis result");
        assert_eq!(response.generation, current);
        let report = response.result.expect("report");
        assert_eq!(report.input.file_name, "part.nc");
        assert!(worker.is_alive());
    }
}
