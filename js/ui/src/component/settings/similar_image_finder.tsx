import { Job } from "semantic/dist/core";
import { FACTOR_ID, SemanticImage } from "semantic/dist/schema";
import { Box } from "solid-bulma";
import { createSignal, For, JSX, onCleanup, onMount, Show } from "solid-js";
import { useApi } from "../../context";
import { blobImageUrl } from "../../semantic/base/image";
import { Notification } from "../bulma/notification";
import { createForm } from "../form";
import { FormButtons } from "../form/FormButtons";
import { InputFloatField } from "../form/InputFloatField";
import { InputIntField } from "../form/InputIntField";
import { renderJobStatus } from "../job/JobStatusLoader";
import { GenericPage } from "../util";
import {
  createLoader,
  FallibleResourceLoader,
  LoaderView,
} from "../util/load";

interface FormData {
  similarityMin: number;
  similarityMax: number;
  maxResults: number;
}

export function SimilarImageFinder(): JSX.Element {
  const api = useApi();

  const [filter, setFilter] = createSignal<FormData | undefined>();

  const form = createForm<FormData>({
    initialValues: {
      similarityMin: 80,
      similarityMax: 100,
      maxResults: 100,
    },
    // validate: (values) => {
    //   const min = tryParseFloat(values.similarityMin);
    //   const max = tryParseFloat(values.similarityMax);
    //   if (min === null || max === null) {
    //     return {
    //       form: { errors: [{ message: 'Severity must be a number.' }] }
    //     };
    //   } else {
    //     return null;
    //   }
    // },
    onSubmit: (values) => {
      setFilter({
        similarityMax: values.similarityMax / 100,
        similarityMin: values.similarityMin / 100,
        maxResults: values.maxResults,
      });
    },
  });

  return (
    <Show
      when={filter()}
      fallback={() => {
        return (
          <Box>
            <form
              onsubmit={(e) => {
                e.preventDefault();
                form.submit();
              }}
            >
              <InputFloatField
                field={form.field("similarityMin")}
                label={"Minimum similarity"}
                min={0}
                max={100}
              />
              <InputFloatField
                field={form.field("similarityMax")}
                label={"Maximum similarity"}
                min={0}
                max={100}
              />
              <InputIntField
                field={form.field("maxResults")}
                label={"Maximum number of results"}
              />

              <FormButtons form={form} />
            </form>
          </Box>
        );
      }}
    >
      {(filter) => {
        return (
          <FallibleResourceLoader
            load={() =>
              api.findSimilarImages(
                filter.maxResults,
                filter.similarityMin,
                filter.similarityMax
              )
            }
          >
            {(job: Job) => <SimilarImageFinderProgress job={job} />}
          </FallibleResourceLoader>
        );
      }}
    </Show>
  );
}

interface SimilarImageMatch {
  image: SemanticImage;
  similarity: number;
}

interface SimilarImageMatches {
  image: SemanticImage;
  related: SimilarImageMatch[];
}

export function SimilarImageFinderProgress(props: { job: Job }): JSX.Element {
  const api = useApi();
  const [jobStatus, setJobStatus] = createLoader<Job | undefined>();
  const [matches, setMatches] = createSignal<SimilarImageMatches[]>([]);

  const done = { done: false };

  onCleanup(() => {
    done.done = true;
  });
  onMount(() => {
    (async () => {
      let job: Job | undefined;
      while (!done.done) {
        try {
          job = await api.jobStatus(props.job.id);
          const newMatches = (await api.jobEvents(props.job.id)).map(
            (ev) => ev.data
          ) as SimilarImageMatches[];

          setJobStatus({ state: "success", data: job });
          setMatches((matches) => [...matches, ...newMatches]);

          if ("Finished" in job.status) {
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
        {(job) => {
          return <div>{renderJobStatus(job)}</div>;
        }}
      </LoaderView>
      <hr />

      <For
        each={matches()}
        fallback={<Notification>No matches found yet...</Notification>}
      >
        {renderMatches}
      </For>
    </div>
  );
}

function renderMatches(matches: SimilarImageMatches): JSX.Element {
  const mainUrl = blobImageUrl(matches.image[FACTOR_ID]);
  return (
    <Box>
      <div>
        <img src={mainUrl} />
      </div>
      <hr />
      <div>
        <For each={matches.related}>
          {(related) => {
            const otherUrl = blobImageUrl(related.image[FACTOR_ID]);
            return (
              <div>
                <div>{related.similarity}</div>
                <div>
                  <img src={otherUrl} />
                </div>
              </div>
            );
          }}
        </For>
      </div>
    </Box>
  );
}

export function SimilarFileFinderPage(): JSX.Element {
  return (
    <GenericPage title={"Find Duplicate Files"}>
      <SimilarImageFinder />
    </GenericPage>
  );
}
