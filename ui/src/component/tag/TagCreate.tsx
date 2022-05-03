import { JSX } from "solid-js/jsx-runtime";
import { useApi } from "../../context";
import { SemanticTag } from "../../semantic/schema";
import { Subtitle } from "../bulma/title";
import { TagForm } from "./TagForm";

export interface TagCreateFormProps {
  onCreated?: (tag: SemanticTag) => void;
  onCancel?: () => void;
}

export function TagCreate(props: TagCreateFormProps): JSX.Element {
  const api = useApi();

  return (
    <div>
      <Subtitle size="is-5">Create Tag</Subtitle>

      <TagForm
        submitLabel="Create"
        resetOnSubmit={true}
        onCancel={props.onCancel}
        onSubmit={(values) =>
          api.tagCreate({ name: values.name }).then((tag) => {
            props.onCreated?.(tag);
          })
        }
      />
    </div>
  );
}
