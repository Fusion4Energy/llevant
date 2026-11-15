//! Work on threads of their own, the same way for every kind: loading a
//! model, opening a universe, reading the input's cards or a field, the
//! analyses ([`crate::jobs`]). A [`Task`] has a [`Progress`] to show and
//! cancel it; what it sends comes back on a channel the UI thread polls
//! once a frame, and the window is woken whenever it sends. Dropping a task
//! cancels it, so starting one of a kind in place of another (a new load,
//! a new volume job) stops the one before, which then sends nothing more
//! anyone reads.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

use llevant::Progress;

/// A worker that does only the latest of the jobs sent to it: those sent
/// while it worked are skipped but for the last (what is under a moving
/// pointer: only where it is now matters).
pub struct Latest<T> {
    tx: Sender<Box<dyn FnOnce() -> T + Send>>,
    rx: Receiver<T>,
}

impl<T: Send + 'static> Latest<T> {
    /// A thread named `name`; `wake` (the window) is asked to repaint
    /// after each result.
    pub fn new(name: &str, wake: Option<egui::Context>) -> Latest<T> {
        let (tx, jobs) = channel::<Box<dyn FnOnce() -> T + Send>>();
        let (results, rx) = channel();
        std::thread::Builder::new()
            .name(format!("llevant-{name}"))
            .spawn(move || {
                // Ends when the sender is dropped.
                while let Ok(mut job) = jobs.recv() {
                    while let Ok(next) = jobs.try_recv() {
                        job = next;
                    }
                    if results.send(job()).is_err() {
                        break;
                    }
                    if let Some(w) = &wake {
                        w.request_repaint();
                    }
                }
            })
            .expect("a thread");
        Latest { tx, rx }
    }

    pub fn send(&self, job: impl FnOnce() -> T + Send + 'static) {
        let _ = self.tx.send(Box::new(job));
    }

    /// The last result made since the last call.
    pub fn latest(&self) -> Option<T> {
        self.rx.try_iter().last()
    }
}

/// A thread pool for the little pixel-sized work a frame must wait for
/// (composing the slice's image): apart from rayon's global pool, which
/// the tracers keep busy, so that the frame does not queue behind them.
pub fn ui_pool() -> &'static rayon::ThreadPool {
    static POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let n = std::thread::available_parallelism().map_or(2, |n| n.get());
        rayon::ThreadPoolBuilder::new()
            .num_threads((n / 2).max(2))
            .thread_name(|k| format!("llevant-ui-{k}"))
            .build()
            .expect("a thread pool")
    })
}

/// A thread pool for long work in the background while the model is shown
/// (building the octrees of a staged model): a thread per core, each at a
/// lower priority than the rest of the process, so that the views'
/// previews and tracers (rayon's global pool) come first and the build
/// takes the time they leave.
pub fn background_pool() -> &'static rayon::ThreadPool {
    static POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .thread_name(|k| format!("llevant-background-{k}"))
            .start_handler(|_| lower_priority())
            .build()
            .expect("a thread pool")
    })
}

/// Lower the calling thread's priority (below normal; on Linux, nice 10),
/// where the system lets a thread have one of its own.
fn lower_priority() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
        };
        // SAFETY: the pseudo-handle of the calling thread, always valid.
        let ok = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL) };
        if ok == 0 {
            log::debug!("could not lower a background thread's priority");
        }
    }
    #[cfg(target_os = "linux")]
    {
        // On Linux a thread has a nice value of its own: 0 is this thread.
        // SAFETY: no pointers; only this thread's priority changes.
        if unsafe { libc::setpriority(libc::PRIO_PROCESS as _, 0, 10) } != 0 {
            log::debug!("could not lower a background thread's priority");
        }
    }
}

/// Work running on a thread of its own, sending values of `T`.
pub struct Task<T> {
    progress: Arc<Progress>,
    started: Instant,
    rx: Receiver<T>,
    /// Set when the thread ends (after its last value), panics included.
    ended: Arc<AtomicBool>,
}

/// Sets its flag when dropped: at the end of a task's thread, however it
/// ends.
struct Ends(Arc<AtomicBool>);

impl Drop for Ends {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

impl<T: Send + 'static> Task<T> {
    /// Run `work` on a thread named `name`; it sends what it has with its
    /// second argument, as often as it likes (volumes so far, then the
    /// volumes), and should stop soon after its progress is cancelled.
    /// `wake` (the window) is asked to repaint after each value.
    pub fn spawn(
        name: &str,
        wake: Option<egui::Context>,
        work: impl FnOnce(&Progress, &dyn Fn(T)) + Send + 'static,
    ) -> Task<T> {
        let progress = Arc::new(Progress::new());
        let (tx, rx) = channel();
        let p = progress.clone();
        let ended = Arc::new(AtomicBool::new(false));
        let ends = Ends(ended.clone());
        std::thread::Builder::new()
            .name(format!("llevant-{name}"))
            .spawn(move || {
                let _ends = ends;
                let send = |v: T| {
                    // Nobody reads it once the task is dropped.
                    let _ = tx.send(v);
                    if let Some(w) = &wake {
                        w.request_repaint();
                    }
                };
                work(&p, &send);
            })
            .expect("a thread");
        Task {
            progress,
            started: Instant::now(),
            rx,
            ended,
        }
    }

    /// [`Task::spawn`] for work with one result: what `work` returns.
    pub fn run(
        name: &str,
        wake: Option<egui::Context>,
        work: impl FnOnce(&Progress) -> T + Send + 'static,
    ) -> Task<T> {
        Task::spawn(name, wake, move |p, send| send(work(p)))
    }
}

impl<T> Task<T> {
    /// What it sent since the last call, in order.
    pub fn poll(&self) -> Vec<T> {
        self.rx.try_iter().collect()
    }

    /// The next value sent, if there is one yet (for work with one result).
    pub fn try_take(&self) -> Option<T> {
        self.rx.try_recv().ok()
    }

    /// Whether its thread has ended: everything it sent can be polled (a
    /// task with one result that ended without it failed).
    pub fn ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }

    pub fn progress(&self) -> &Arc<Progress> {
        &self.progress
    }

    pub fn started(&self) -> Instant {
        self.started
    }

    /// Ask it to stop (it says when it has, by what it sends).
    pub fn cancel(&self) {
        self.progress.cancel();
    }
}

impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        self.progress.cancel();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn wait<T>(t: &Task<T>) -> T {
        let start = Instant::now();
        loop {
            if let Some(v) = t.try_take() {
                return v;
            }
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn values_come_in_order() {
        let t = Task::spawn("test", None, |_, send| {
            for k in 0..5 {
                send(k);
            }
        });
        let got: Vec<i32> = (0..5).map(|_| wait(&t)).collect();
        assert_eq!(got, [0, 1, 2, 3, 4]);
    }

    #[test]
    fn what_was_sent_is_there_once_ended() {
        let t = Task::run("test", None, |_| 7);
        let start = Instant::now();
        while !t.ended() {
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(t.try_take(), Some(7));
        // A thread that panics ends too, with nothing sent.
        let t: Task<i32> = Task::run("test", None, |_| panic!("on purpose"));
        while !t.ended() {
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(t.try_take(), None);
    }

    #[test]
    fn latest_skips_what_it_had_no_time_for() {
        let l = Latest::new("test", None);
        let (go, wait) = channel::<()>();
        let (started, has_started) = channel::<()>();
        // The first job holds the worker while the others queue.
        l.send(move || {
            let _ = started.send(());
            let _ = wait.recv();
            0
        });
        has_started.recv().expect("the first job runs");
        for k in 1..=5 {
            l.send(move || k);
        }
        let _ = go.send(());
        let start = Instant::now();
        let mut got = Vec::new();
        while got.last() != Some(&5) {
            assert!(start.elapsed() < Duration::from_secs(10));
            got.extend(l.rx.try_iter());
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(got, [0, 5]);
    }

    #[test]
    fn dropping_a_task_cancels_it() {
        let (seen, saw) = channel();
        let t = Task::run("test", None, move |p: &Progress| {
            while !p.cancelled() {
                std::thread::sleep(Duration::from_millis(1));
            }
            let _ = seen.send(());
        });
        drop(t);
        saw.recv_timeout(Duration::from_secs(10))
            .expect("the work saw it was cancelled");
    }
}
