use std::any::Any;

use futures::{FutureExt, future::BoxFuture};

use crate::DbError;

type BlockingOutput = Box<dyn Any + Send>;
type BlockingOp = Box<dyn FnOnce() -> std::result::Result<BlockingOutput, DbError> + Send>;

pub trait AsyncRuntime: Send + Sync + 'static {
    fn spawn_blocking_erased(
        &self,
        op: BlockingOp,
    ) -> BoxFuture<'static, std::result::Result<BlockingOutput, DbError>>;

    fn spawn_blocking<R, F>(&self, op: F) -> BoxFuture<'static, std::result::Result<R, DbError>>
    where
        Self: Sized,
        R: Send + 'static,
        F: FnOnce() -> std::result::Result<R, DbError> + Send + 'static,
    {
        spawn_blocking_on(self, op)
    }
}

pub fn spawn_blocking_on<R, F>(
    runtime: &dyn AsyncRuntime,
    op: F,
) -> BoxFuture<'static, std::result::Result<R, DbError>>
where
    R: Send + 'static,
    F: FnOnce() -> std::result::Result<R, DbError> + Send + 'static,
{
    let erased = runtime.spawn_blocking_erased(Box::new(move || {
        op().map(|value| Box::new(value) as BlockingOutput)
    }));
    async move {
        let value = erased.await?;
        value.downcast::<R>().map(|value| *value).map_err(|_| {
            DbError::Storage("blocking runtime returned unexpected result type".to_string())
        })
    }
    .boxed()
}

#[derive(Debug, Default, Clone, Copy)]
pub struct InlineAsyncRuntime;

impl AsyncRuntime for InlineAsyncRuntime {
    fn spawn_blocking_erased(
        &self,
        op: BlockingOp,
    ) -> BoxFuture<'static, std::result::Result<BlockingOutput, DbError>> {
        async move { op() }.boxed()
    }
}

#[cfg(feature = "tokio")]
#[derive(Debug, Default, Clone, Copy)]
pub struct TokioAsyncRuntime;

#[cfg(feature = "tokio")]
impl AsyncRuntime for TokioAsyncRuntime {
    fn spawn_blocking_erased(
        &self,
        op: BlockingOp,
    ) -> BoxFuture<'static, std::result::Result<BlockingOutput, DbError>> {
        async move {
            tokio::task::spawn_blocking(op)
                .await
                .map_err(|err| DbError::Storage(format!("tokio spawn_blocking failed: {err}")))?
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_runtime_impl<T: AsyncRuntime>() {}

    #[test]
    fn inline_runtime_impl_compiles() {
        assert_runtime_impl::<InlineAsyncRuntime>();
    }

    #[cfg(feature = "tokio")]
    #[test]
    fn tokio_runtime_impl_compiles() {
        assert_runtime_impl::<TokioAsyncRuntime>();
    }
}
