use brass::{
    component::{msg::MsgComponent, Component, Context},
    dom::{builder::div, Render, View},
    effect::{set_timeout, EffectGuard, TimeoutGuard},
    signal::signal::{Mutable, SignalExt},
};
use factdb::{ClassContainer, Id};
use semantic_core::{
    api::{self, Job, JobId},
    base::Video,
};

use crate::{
    components::{
        loader::spinner,
        util::{
            box_, notification_default, notification_error, notification_success, subtitle_4,
            ButtonBuilder,
        },
    },
    context,
};

use super::optimise_compare::FileOptimiseCompare;

pub struct VideoOptimiser {
    pub video_id: Id,
}

impl Render for VideoOptimiser {
    fn render(self) -> View {
        State::build(self)
    }
}

#[derive(Clone)]
enum Phase {
    Idle,
    JobRunning { id: JobId, job: Option<Job> },
    Loading,
    VideoLoaded(Video),
    Failed(String),
}

struct State {
    video_id: Id,
    phase: Mutable<Phase>,

    guard: Option<EffectGuard>,
    job_poll_guard: Option<TimeoutGuard>,
}

#[derive(Debug)]
enum Msg {
    Start,
    JobStartLoaded(Result<api::OptimiseVideoReply, anyhow::Error>),
    JobPollTick,
    JobPolled(Result<Job, anyhow::Error>),
    VideoLoaded(Result<Video, anyhow::Error>),
}

impl State {
    fn load_job(&mut self, job_id: JobId, ctx: Context<Self>) {
        self.guard = Some(ctx.spawn_map(
            async move { context::api().job(job_id).await },
            Msg::JobPolled,
        ));
    }
}

impl MsgComponent for State {
    type Properties = VideoOptimiser;
    type Msg = Msg;

    fn init(props: Self::Properties, _ctx: brass::component::Context<Self>) -> Self {
        Self {
            phase: Mutable::new(Phase::Idle),
            video_id: props.video_id,
            guard: None,
            job_poll_guard: None,
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: brass::component::Context<Self>) {
        tracing::trace!(msg=?msg, "optimiser msg");
        match msg {
            Msg::Start => {
                let video_id = self.video_id;
                let guard = ctx.spawn(async move {
                    let out = context::api()
                        .optimise_video(api::OptimiseVideo { video_id })
                        .await;
                    Msg::JobStartLoaded(out)
                });
                self.guard = Some(guard);
            }
            Msg::JobStartLoaded(res) => match res {
                Ok(reply) => {
                    self.phase.set(Phase::JobRunning {
                        id: reply.job_id,
                        job: None,
                    });
                    self.load_job(reply.job_id, ctx);
                }
                Err(err) => {
                    self.phase.set(Phase::Failed(err.to_string()));
                }
            },
            Msg::JobPollTick => {
                let phase: Phase = self.phase.lock_ref().clone();
                if let Phase::JobRunning { id, job: _ } = phase {
                    self.load_job(id.clone(), ctx);
                }
            }
            Msg::JobPolled(res) => {
                let new_phase = match res {
                    Ok(job) => match &job.status {
                        api::JobStatus::Queued { .. } | api::JobStatus::Running { .. } => {
                            Phase::JobRunning {
                                id: job.id,
                                job: Some(job),
                            }
                        }
                        api::JobStatus::Finished { result: Ok(_res) } => {
                            let id = self.video_id;

                            self.guard = Some(ctx.spawn_map(
                                async move {
                                    let raw = context::api().entity(id).await?;
                                    let video = Video::try_from_map(raw)?;
                                    Ok::<Video, anyhow::Error>(video)
                                },
                                Msg::VideoLoaded,
                            ));

                            Phase::Loading
                        }
                        api::JobStatus::Finished { result: Err(err) } => {
                            Phase::Failed(err.message.clone())
                        }
                    },
                    Err(err) => Phase::Failed(err.to_string()),
                };
                self.phase.set(new_phase);

                let handle = ctx.handle();
                self.job_poll_guard =
                    Some(set_timeout(std::time::Duration::from_secs(1), move || {
                        handle.send(Msg::JobPollTick);
                    }));
            }
            Msg::VideoLoaded(res) => {
                self.phase.set(match res {
                    Ok(video) => Phase::VideoLoaded(video),
                    Err(err) => Phase::Failed(err.to_string()),
                });
            }
        }
    }

    fn render(&mut self, ctx: brass::component::Context<Self>) -> brass::dom::TagBuilder {
        let handle = ctx.handle();
        let sig = self.phase.signal_cloned().map(move |phase| {
            let inner = match phase {
                Phase::Idle => ButtonBuilder::new()
                    .label("Optimise")
                    .on(handle.callback(|| Msg::Start))
                    .build()
                    .into_view(),
                Phase::JobRunning { id: _, job: None } => div().and(spinner()).into_view(),
                Phase::JobRunning {
                    id: _,
                    job: Some(job),
                } => {
                    let content = match &job.status {
                        semantic_core::api::JobStatus::Queued { queue_position: _ } => {
                            notification_default().and("Waiting for job to start...")
                        }
                        semantic_core::api::JobStatus::Running {
                            step,
                            progress_message,
                            progress_percent,
                        } => {
                            let start_time = job
                                .started_at
                                .as_ref()
                                .map(|t| t.to_datetime().to_string())
                                .unwrap_or_default();

                            let progress = match (progress_percent, progress_message) {
                                (Some(pct), Some(msg)) => format!("{pct}% | {msg}"),
                                (Some(pct), None) => format!("{pct}%"),
                                (None, Some(msg)) => format!("{msg}"),
                                (None, None) => "n/a".to_string(),
                            };

                            let progress = div().and("Progress: ").and(&progress);
                            let time = div().and("Since: ").and(start_time);

                            let name = div().and(step.as_ref().unwrap_or(&job.name));
                            notification_default().and(name).and(time).and(progress)
                        }
                        semantic_core::api::JobStatus::Finished { result } => match result {
                            Ok(_) => notification_success().and("Complete!"),
                            Err(err) => {
                                notification_error().and(format!("Job failed: {}", err.message))
                            }
                        },
                    };

                    div().and(div().and(spinner())).and(content).into_view()
                }
                Phase::Loading => spinner().into_view(),
                Phase::VideoLoaded(video) => FileOptimiseCompare {
                    file: video.clone(),
                    // TODO: add cancel button + callback here (also for previous optimisation)
                    on_cancel: None,
                }
                .render(),
                Phase::Failed(err) => notification_error().and(err.to_string()).into_view(),
            };

            div().and(inner)
        });

        box_().and(subtitle_4().and("Optimise Video")).signal(sig)
    }
}
