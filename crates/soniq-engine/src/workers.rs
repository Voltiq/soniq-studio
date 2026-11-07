//! Worker threads that share the audio thread's work. Each dispatch hands
//! out jobs (tracks) one at a time to whichever thread is free, the audio
//! thread included, so a worker that is slow to wake only means the audio
//! thread does more of them itself. Nothing locks or allocates: a job is
//! claimed with one atomic, and idle workers spin briefly before sleeping.

use std::hint::spin_loop;
use std::sync::Arc;
use std::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed, Release};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize};
use std::thread::{self, JoinHandle, Thread};

/// More jobs than this in one dispatch run on the calling thread alone.
const MAX_JOBS: usize = 0xFFFF;
/// How long an idle worker spins for the next dispatch before it sleeps
/// (dispatches come in bursts: several per audio block).
const SPINS: u32 = 1 << 14;

/// One dispatch: `run(i)` for every job `i`.
struct Job<'a> {
    run: &'a (dyn Fn(usize) + Sync),
}

struct Shared {
    /// The dispatch's generation (high 32 bits), the next job to claim
    /// (16 bits) and how many there are (16 bits), so a claim checks all
    /// three at once and a late worker never claims a job of a dispatch it
    /// did not see begin.
    state: AtomicU64,
    job: AtomicPtr<Job<'static>>,
    done: AtomicUsize,
    /// A dispatch is running (a second caller runs its jobs alone).
    busy: AtomicBool,
    stop: AtomicBool,
}

const NEXT: u64 = 1 << 16;

fn generation(state: u64) -> u32 {
    (state >> 32) as u32
}

fn next(state: u64) -> usize {
    ((state >> 16) & 0xFFFF) as usize
}

fn count(state: u64) -> usize {
    (state & 0xFFFF) as usize
}

impl Shared {
    /// Claims and runs jobs of dispatch `round` until none are left.
    fn claim(&self, round: u32, job: *const Job<'static>) {
        let mut state = self.state.load(Acquire);
        loop {
            if generation(state) != round || next(state) >= count(state) {
                return;
            }
            match self.state.compare_exchange_weak(state, state + NEXT, AcqRel, Acquire) {
                Ok(_) => {
                    // SAFETY: the claim succeeded, so dispatch `round` still has
                    // a job unfinished and its caller is still waiting in
                    // `run`, keeping the job alive. `job` was read after
                    // `round` was published, and a later dispatch cannot have
                    // replaced it before this one finished.
                    unsafe { ((*job).run)(next(state)) };
                    self.done.fetch_add(1, Release);
                    state = self.state.load(Acquire);
                }
                Err(now) => state = now,
            }
        }
    }
}

pub struct Workers {
    shared: Arc<Shared>,
    threads: Vec<(Thread, Option<JoinHandle<()>>)>,
}

impl Workers {
    /// Starts `count` worker threads (zero runs everything on the caller).
    pub fn new(count: usize) -> Self {
        let shared = Arc::new(Shared {
            state: AtomicU64::new(0),
            job: AtomicPtr::new(std::ptr::null_mut()),
            done: AtomicUsize::new(0),
            busy: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        });
        let threads = (0..count)
            .filter_map(|i| {
                let shared = Arc::clone(&shared);
                let handle = thread::Builder::new().name(format!("soniq-dsp-{}", i + 1)).spawn(move || work(&shared)).ok()?;
                Some((handle.thread().clone(), Some(handle)))
            })
            .collect();
        Self { shared, threads }
    }

    /// Worker threads besides the caller's.
    pub fn len(&self) -> usize {
        self.threads.len()
    }

    pub fn is_empty(&self) -> bool {
        self.threads.is_empty()
    }

    /// Runs `job(i)` for every `i` in `0..jobs`, spread over the workers and
    /// the calling thread; returns when all have finished.
    pub fn run(&self, jobs: usize, job: &(dyn Fn(usize) + Sync)) {
        let s = &*self.shared;
        if !(2..=MAX_JOBS).contains(&jobs) || self.threads.is_empty() || s.busy.swap(true, Acquire) {
            (0..jobs).for_each(job);
            return;
        }
        let dispatch = Job { run: job };
        let ptr = (&raw const dispatch).cast_mut().cast::<Job<'static>>();
        s.done.store(0, Relaxed);
        s.job.store(ptr, Relaxed);
        let round = generation(s.state.load(Relaxed)).wrapping_add(1);
        s.state.store((u64::from(round) << 32) | jobs as u64, Release);
        for (thread, _) in &self.threads {
            thread.unpark();
        }
        s.claim(round, ptr);
        // The last jobs may still be running on workers.
        while s.done.load(Acquire) < jobs {
            spin_loop();
        }
        s.busy.store(false, Release);
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        self.shared.stop.store(true, Release);
        for (thread, handle) in &mut self.threads {
            thread.unpark();
            if let Some(handle) = handle.take() {
                let _ = handle.join();
            }
        }
    }
}

fn work(s: &Shared) {
    raise_priority();
    let mut seen = generation(s.state.load(Acquire));
    loop {
        let mut spins = 0;
        let round = loop {
            if s.stop.load(Acquire) {
                return;
            }
            let round = generation(s.state.load(Acquire));
            if round != seen {
                break round;
            }
            if spins < SPINS {
                spins += 1;
                spin_loop();
            } else {
                // Woken by the next dispatch (or a stray wake-up: look again).
                thread::park();
            }
        };
        seen = round;
        s.claim(round, s.job.load(Acquire));
    }
}

/// Workers run at the audio thread's priority, so the scheduler never keeps
/// one from finishing a job the audio thread is waiting for.
#[cfg(windows)]
fn raise_priority() {
    use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL};
    // SAFETY: changes only this thread's own priority.
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) };
}

#[cfg(not(windows))]
fn raise_priority() {}

/// Worker threads to start for `cores` cores of processing: the audio thread
/// is one of them. `None` uses every core but one, up to eight.
pub fn worker_count(cores: Option<u32>) -> usize {
    let available = thread::available_parallelism().map_or(1, usize::from);
    let wanted = cores.map_or(available.saturating_sub(1).clamp(1, 8), |c| c as usize);
    wanted.min(available).saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    #[test]
    fn every_job_runs_exactly_once_on_every_dispatch() {
        let workers = Workers::new(3);
        let hits: Vec<AtomicU32> = (0..37).map(|_| AtomicU32::new(0)).collect();
        for round in 1..=5000 {
            let jobs = 2 + round % 36;
            workers.run(jobs, &|i| {
                hits[i].fetch_add(1, Relaxed);
            });
            for (i, h) in hits.iter().enumerate() {
                assert_eq!(h.swap(0, Relaxed), u32::from(i < jobs), "round {round}, job {i}");
            }
        }
    }

    #[test]
    fn jobs_run_side_by_side() {
        let workers = Workers::new(3);
        let threads = std::sync::Mutex::new(std::collections::HashSet::new());
        for _ in 0..50 {
            workers.run(8, &|_| {
                threads.lock().unwrap().insert(thread::current().id());
                std::thread::sleep(std::time::Duration::from_millis(1));
            });
        }
        assert!(threads.lock().unwrap().len() > 1, "more than the calling thread did the work");
    }
}
