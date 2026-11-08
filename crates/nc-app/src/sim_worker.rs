//! Background seek worker.
//!
//! A long scrub rebuilds a deterministic simulation and seeks it on a named
//! thread. A monotonic generation counter makes every superseded request inert:
//! the worker drains queued jobs before computing and refuses to deliver a
//! result whose generation is no longer current.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crossbeam_channel::{Receiver, Sender};
use nc_core::ToolTable;
use nc_gcode::Program;
use nc_sim::{SimConfig, Simulation};
use nc_stock::StockModel;

/// One request to rebuild and seek a simulation.
#[derive(Debug)]
struct SeekJob {
    generation: u64,
    program: Arc<Program>,
    stock: StockModel,
    tools: ToolTable,
    config: SimConfig,
    target_s: f64,
}

/// One completed seek. A generation that is no longer current is discarded by
/// the shell before it replaces state.
pub struct SeekResponse {
    /// Generation this result belongs to.
    pub generation: u64,
    /// The rebuilt simulation, or a plain-language construction error.
    pub result: Result<Simulation, String>,
}

/// Handle to the long-seek thread.
pub struct SeekWorker {
    jobs: Sender<SeekJob>,
    results: Receiver<SeekResponse>,
    latest: Arc<AtomicU64>,
    alive: Arc<AtomicBool>,
    generation: u64,
}

impl SeekWorker {
    /// Spawns the worker thread.
    pub fn spawn() -> Result<Self, String> {
        let (job_tx, job_rx) = crossbeam_channel::unbounded::<SeekJob>();
        let (result_tx, result_rx) = crossbeam_channel::unbounded::<SeekResponse>();
        let latest = Arc::new(AtomicU64::new(0));
        let alive = Arc::new(AtomicBool::new(true));
        let worker_latest = Arc::clone(&latest);
        let worker_alive = Arc::clone(&alive);
        std::thread::Builder::new()
            .name("nca-sim-seek".to_string())
            .spawn(move || worker_loop(&job_rx, &result_tx, &worker_latest, &worker_alive))
            .map_err(|error| format!("could not start the seek worker: {error}"))?;
        Ok(Self {
            jobs: job_tx,
            results: result_rx,
            latest,
            alive,
            generation: 0,
        })
    }

    /// Queues a seek and returns its generation.
    pub fn request(
        &mut self,
        program: Arc<Program>,
        stock: StockModel,
        tools: ToolTable,
        config: SimConfig,
        target_s: f64,
    ) -> Result<u64, String> {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.latest.store(generation, Ordering::Release);
        self.jobs
            .send(SeekJob {
                generation,
                program,
                stock,
                tools,
                config,
                target_s,
            })
            .map_err(|_| "the seek worker stopped unexpectedly".to_string())?;
        Ok(generation)
    }

    /// Marks every queued or running result stale.
    pub fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.latest.store(self.generation, Ordering::Release);
    }

    /// Non-blocking receive of one finished seek.
    #[must_use]
    pub fn try_recv(&self) -> Option<SeekResponse> {
        self.results.try_recv().ok()
    }

    /// `false` after the worker thread exits, so a stuck seek can be reported.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }
}

fn worker_loop(
    jobs: &Receiver<SeekJob>,
    results: &Sender<SeekResponse>,
    latest: &AtomicU64,
    alive: &AtomicBool,
) {
    while let Ok(mut job) = jobs.recv() {
        while let Ok(newer) = jobs.try_recv() {
            job = newer;
        }
        if job.generation != latest.load(Ordering::Acquire) {
            continue;
        }
        let result = Simulation::new(&job.program, job.stock, &job.tools, &job.config)
            .map_err(|error| error.to_string())
            .map(|mut simulation| {
                simulation.seek_to_time(job.target_s);
                simulation
            });
        if job.generation != latest.load(Ordering::Acquire) {
            continue;
        }
        if results
            .send(SeekResponse {
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
    fn seek_worker_delivers_only_the_latest_generation() {
        let source = "G21 G90\nG1 X100 F1000\nG1 X0\nG1 X100\n";
        let program = Arc::new(nc_gcode::parse_program(source).expect("program"));
        let stock = StockModel::infer(&program);
        let tools = ToolTable::default();
        let config = SimConfig::default();
        let mut worker = SeekWorker::spawn().expect("worker");

        let _stale = worker
            .request(
                Arc::clone(&program),
                stock,
                tools.clone(),
                config.clone(),
                0.05,
            )
            .expect("stale request");
        let current = worker
            .request(program, stock, tools, config, 0.4)
            .expect("current request");

        let mut accepted = None;
        for _ in 0..200 {
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
        let response = accepted.expect("latest seek result");
        assert_eq!(response.generation, current);
        let simulation = response.result.expect("simulation");
        assert!(simulation.snapshot().time_s > 0.0);
        assert!(simulation.snapshot().time_s < 1.0);
        assert!(worker.is_alive());
    }
}
