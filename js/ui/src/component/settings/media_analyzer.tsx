import { Job } from "semantic/dist/core";
import {
  createEffect,
  createSignal,
  JSX,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import { useApi } from "../../context";
import { Button } from "../bulma/button";
import { Notification, NotificationError } from "../bulma/notification";
import { GenericPage } from "../util";
import {
  createFallibleResource,
  FallibleResource,
  FallibleResourceLoader,
} from "../util/load";

export function MediaAnalyzer(): JSX.Element {
  const api = useApi();
  const [started, setStarted] = createSignal(false);

  return (
    <div>
      <Show
        when={started()}
        fallback={
          <Button size="is-large" onclick={() => setStarted(true)}>
            Analyze all media
          </Button>
        }
      >
        <FallibleResourceLoader load={() => api.startMediaAnalysis()}>
          {(job: Job) => <MediaAnalysisProgress job={job} />}
        </FallibleResourceLoader>
      </Show>
    </div>
  );
}

function MediaAnalysisProgress(props: { job: Job }): JSX.Element {
  const api = useApi();
  const [job, actions] = createFallibleResource<Job>(() =>
    api.jobStatus(props.job.id)
  );

  let fetchStatus = { stopped: false };
  const onTimeout = () => {
    if (!fetchStatus.stopped) {
      actions.refetch();
      setTimeout(onTimeout, 1000);
    }
  };

  onCleanup(() => {
    fetchStatus.stopped = true;
  });
  onMount(() => {
    onTimeout();
  });

  return (
    <div>
      <FallibleResource<Job> resource={job}>
        {(job) => {
          const status = job.status;
          console.debug({ status });
          if ("Queued" in status) {
            return <Notification>Queued...</Notification>;
          } else if ("Running" in status) {
            const run = status.Running;
            const percent = run.progress_percent
              ? ` (${run.progress_percent}%)`
              : "";
            const msg = run.progress_message ?? "";

            return (
              <Notification>
                Progress: {msg} {percent}
              </Notification>
            );
          } else if ("Finished" in status) {
            fetchStatus.stopped = true;
            const fin = status.Finished;
            if ("Ok" in fin.result) {
              return (
                <Notification color="is-success">
                  Finished successfully!
                  <br />
                  {fin.result.Ok}
                </Notification>
              );
            } else if ("Err" in fin.result) {
              return (
                <NotificationError>
                  <b>Failed!</b>
                  {fin.result.Err.message}
                </NotificationError>
              );
            }
          } else {
            console.error(status);
            throw new Error("invalid!");
          }
        }}
      </FallibleResource>
    </div>
  );
}

export function MediaAnalyzerPage(): JSX.Element {
  return (
    <GenericPage title="Media Analyzer">
      <MediaAnalyzer />
    </GenericPage>
  );
}
