use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use semantic_app::{AppRequestContext, AppSession, Principal, SemanticApp};
use tokio::sync::{Mutex, Notify, Semaphore};

pub struct Resources {
    pub app: SemanticApp,
    pub session: Arc<AppSession>,
}

pub struct Lifecycle {
    closing: AtomicBool,
    resources: Mutex<Option<Resources>>,
    permits: Arc<Semaphore>,
    permit_count: u32,
    close_result: Mutex<Option<Result<(), String>>>,
    closed: Notify,
}

impl Lifecycle {
    pub fn new(app: SemanticApp, max_concurrent_requests: usize) -> Self {
        let session = app.new_session(format!("node-{}", uuid::Uuid::new_v4()));
        Self {
            closing: AtomicBool::new(false),
            resources: Mutex::new(Some(Resources { app, session })),
            permits: Arc::new(Semaphore::new(max_concurrent_requests)),
            permit_count: max_concurrent_requests as u32,
            close_result: Mutex::new(None),
            closed: Notify::new(),
        }
    }

    pub async fn context(
        &self,
    ) -> Result<(AppRequestContext, tokio::sync::OwnedSemaphorePermit), String> {
        if self.closing.load(Ordering::Acquire) {
            return Err("EMBEDDED_CLOSED: embedded Semantic is closing or closed".to_string());
        }
        let permit = self
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| "EMBEDDED_BUSY: concurrent request limit reached".to_string())?;
        if self.closing.load(Ordering::Acquire) {
            return Err("EMBEDDED_CLOSED: embedded Semantic is closing or closed".to_string());
        }
        let resources = self.resources.lock().await;
        let resources = resources
            .as_ref()
            .ok_or_else(|| "EMBEDDED_CLOSED: embedded Semantic is closed".to_string())?;
        Ok((
            AppRequestContext {
                app: resources.app.clone(),
                principal: Principal::system(),
                session: Some(resources.session.clone()),
                request_scope: None,
            },
            permit,
        ))
    }

    pub async fn close(&self) -> Result<(), String> {
        let leader = !self.closing.swap(true, Ordering::AcqRel);
        if leader {
            let all = self
                .permits
                .clone()
                .acquire_many_owned(self.permit_count)
                .await
                .map_err(|_| "EMBEDDED_CLOSE_FAILED: request gate closed".to_string())?;
            let resources = self.resources.lock().await.take();
            let result = match resources {
                Some(resources) => resources
                    .app
                    .shutdown()
                    .await
                    .map_err(|err| format!("EMBEDDED_CLOSE_FAILED: {err}")),
                None => Ok(()),
            };
            drop(all);
            *self.close_result.lock().await = Some(result.clone());
            self.closed.notify_waiters();
            self.permits.close();
            return result;
        }
        loop {
            let notified = self.closed.notified();
            if let Some(result) = self.close_result.lock().await.clone() {
                return result;
            }
            notified.await;
        }
    }
}
