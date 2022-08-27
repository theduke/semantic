import { Job, JobStatus } from "semantic/dist/core";
import { createEffect, JSX, onCleanup, onMount } from "solid-js";
import { useApi } from "../../context";
import {
  createFallibleResource,
  createLoader,
  FallibleResource,
  LoaderView,
} from "../util/load";
import { Notification, NotificationError } from "../bulma/notification";

export interface JobStatusLoaderProps {
  jobId: string;
  render?: (job: Job) => JSX.Element;
}

export function JobStatusLoader(props: JobStatusLoaderProps): JSX.Element {
  const api = useApi();
  const [jobStatus, setJobStatus] = createLoader<Job | undefined>();

  const done = { done: false };
  onCleanup(() => {
    done.done = true;
  });
  onMount(() => {
    (async () => {
      while (!done.done) {
        try {
          const job = await api.jobStatus(props.jobId);
          setJobStatus({ state: "success", data: job });

          if (job && "Finished" in job?.status) {
            break;
          }
        } catch (err: any) {
          setJobStatus({ state: "error", error: err });
          break;
        }
      }
    })();
  });

  return (
    <div>
      <LoaderView loader={jobStatus}>
        {props.render ?? renderJobStatus}
      </LoaderView>
    </div>
  );
}

export function renderJobStatus(job: Job): JSX.Element {
  const status = job.status;
  console.debug({ status });
  if ("Queued" in status) {
    return <Notification>Queued...</Notification>;
  } else if ("Running" in status) {
    const run = status.Running;
    const percent = run.progress_percent ? ` (${run.progress_percent}%)` : "";
    const msg = run.progress_message ?? "";
    // TODO: show steps / active step
    return (
      <Notification>
        Running: {msg} {percent}
      </Notification>
    );
  } else if ("Finished" in status) {
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
}
