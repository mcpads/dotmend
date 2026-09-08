use dotmend::art::{ArtError, ArtResult};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub struct Invocations {
    slots: Arc<Semaphore>,
    budget: Mutex<(Instant, f64)>,
}
impl Invocations {
    pub fn new() -> Self {
        Self {
            slots: Arc::new(Semaphore::new(32)),
            budget: Mutex::new((Instant::now(), 64.0)),
        }
    }
    pub fn enter(&self) -> ArtResult<OwnedSemaphorePermit> {
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| Self::limited(100))?;
        let mut budget = self
            .budget
            .lock()
            .map_err(|_| ArtError::new("internal_error", "Failed to lock the invocation budget"))?;
        let now = Instant::now();
        budget.1 = (budget.1 + now.duration_since(budget.0).as_secs_f64() * 64.0).min(64.0);
        budget.0 = now;
        if budget.1 < 1.0 {
            return Err(Self::limited(16));
        }
        budget.1 -= 1.0;
        Ok(permit)
    }
    fn limited(delay: u64) -> ArtError {
        let mut error = ArtError::new(
            "rate_limited",
            "Too many calls. Await pending results and retry after the indicated delay",
        )
        .detail(serde_json::json!({"retry_after_ms":delay}));
        error.retryable = true;
        error
    }
}
