pub(crate) fn normalized_worker_count(workers: Option<usize>) -> usize {
    workers.filter(|workers| *workers > 0).unwrap_or(1)
}

pub(crate) fn current_worker_count() -> usize {
    if rayon::current_thread_index().is_some() {
        rayon::current_num_threads()
    } else {
        1
    }
}

pub(crate) fn install<R: Send>(
    workers: Option<usize>,
    operation: impl FnOnce() -> R + Send,
) -> Result<R, rayon::ThreadPoolBuildError> {
    if rayon::current_thread_index().is_some() {
        return Ok(operation());
    }
    rayon::ThreadPoolBuilder::new()
        .num_threads(normalized_worker_count(workers))
        .build()
        .map(|pool| pool.install(operation))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_worker_limit_is_single_threaded() {
        assert_eq!(install(None, rayon::current_num_threads).unwrap(), 1);
    }

    #[test]
    fn nested_work_reuses_the_current_pool() {
        let workers = install(Some(2), || {
            install(Some(8), rayon::current_num_threads).unwrap()
        })
        .unwrap();
        assert_eq!(workers, 2);
    }
}
