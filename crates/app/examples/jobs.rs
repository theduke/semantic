//! `cargo run -p semantic_app --example jobs`: native input, ephemeral typed output,
//! real Redb metadata, progress, cancellation and history cleanup.
use semantic_app::{DbScopeId, Principal, SemanticApp};
use semantic_jobs::{
    JobContext, JobError, JobHandler, JobKindDescriptor, JobKindId, JobProgress, JobsBuilder,
};
use std::{future::Future, pin::Pin, sync::Arc};

struct Compute(JobKindDescriptor);
impl JobHandler for Compute {
    type Input = tokio::sync::oneshot::Receiver<u64>;
    type Output = u64;
    fn kind(&self) -> &JobKindDescriptor {
        &self.0
    }
    fn run<'a>(
        &'a self,
        input: Self::Input,
        context: JobContext,
    ) -> Pin<Box<dyn Future<Output = Result<u64, JobError>> + Send + 'a>> {
        Box::pin(async move {
            let value = tokio::select! {
                value = input => value.map_err(|_| JobError::new("input_closed", "Input channel closed"))?,
                _ = context.cancellation().cancelled() => return Err(JobError::new("cancelled", "Cancelled")),
            };
            context
                .report_progress(JobProgress {
                    completed: 1,
                    total: Some(1),
                    unit: Some("steps".into()),
                    phase: Some("compute".into()),
                })
                .map_err(|e| JobError::new("progress", e.to_string()))?;
            Ok(value * 2)
        })
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let backend = semantic_db_redb::open_backend(
        directory.path().join("jobs.redb"),
        semantic_data::schema::DbOpenMode::AutoCreate,
    )?;
    let mut registry = JobsBuilder::new();
    let compute = registry.register(Compute(JobKindDescriptor {
        id: JobKindId("example.compute".into()),
        title: "Compute".into(),
        description: None,
    }))?;
    let app = SemanticApp::builder()
        .with_default_scope(
            DbScopeId::new("example"),
            Arc::new(semantic_db_core::Db::new(backend)),
        )
        .with_jobs(registry.build(), Default::default())
        .build()?;
    let jobs = app.jobs(&Principal::system(), "example".into()).await?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let ticket = jobs.submit(&compute, receiver, Default::default()).await?;
    sender.send(21).map_err(|_| "receiver closed")?;
    println!("Typed runtime result: {}", ticket.wait().await?);
    let (_sender, receiver) = tokio::sync::oneshot::channel();
    let cancelled = jobs.submit(&compute, receiver, Default::default()).await?;
    jobs.cancel(cancelled.id.clone()).await?;
    println!("Cancelled result: {:?}", cancelled.wait().await);
    println!(
        "Cleared {} operational records",
        jobs.clear_completed().await?.deleted
    );
    app.shutdown().await?;
    Ok(())
}
