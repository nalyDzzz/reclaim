//! Thread pools for long disk walks.

use anyhow::Context;

/// Runs `work`, and any rayon parallelism inside it, on a thread pool of its
/// own instead of rayon's shared one.
///
/// The shared pool only picks up work queued from outside it once its
/// threads have nothing of their own left, so a long walk there (a drive
/// scan, Cleanup's analysis) holds up every other feature's parallel work
/// until it's finished. A drive scan started while Cleanup was still
/// checking used to sit on its first folder that way.
pub fn run_isolated<T: Send>(name: &'static str, work: impl FnOnce() -> T + Send) -> anyhow::Result<T> {
    let pool =
        rayon::ThreadPoolBuilder::new().thread_name(move |i| format!("{name}-{i}")).build().context("Couldn't start worker threads")?;
    Ok(pool.install(work))
}
