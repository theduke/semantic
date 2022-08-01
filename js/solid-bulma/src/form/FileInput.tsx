import { JSX } from "solid-js/jsx-runtime";

export type FileInputSize = "is-small" | "is-normal" | "is-medium" | "is-large";

export interface FileInputProps {
  label?: JSX.Element;
  boxed?: boolean;
  fileName?: string;
  fileNameIsLeft?: boolean;
  fullWidth?: boolean;
  size?: FileInputSize;

  multiple?: boolean;
  // List of allows file types.
  // Either a mime type match like "image/*", 
  // or a file extension including the dot, like ".png".
  //
  // Eg: ['image/*', '.doc', '.docx']
  accept?: string[];

  onChange?: JSX.EventHandler<HTMLInputElement, Event>;
  onFilesChange?: (files: FileList | null) => void;
}

export function FileInput(props: FileInputProps): JSX.Element {
  const label = props.label ?? "Choose a file…";
  const haseFileName = !!props.fileName;

  let classes = "file";
  if (props.boxed) {
    classes += " is-boxed";
  }
  if (haseFileName) {
    classes += " has-name";
    if (props.fileNameIsLeft) {
      classes += " is-right";
    }
  }
  if (props.fullWidth) {
    classes += " is-fullwidth";
  }

  const onChange: JSX.EventHandler<HTMLInputElement, Event> | undefined = (props.onChange || props.onFilesChange) ? (e) => {
    props.onFilesChange?.(e.currentTarget.files);
    props.onChange?.(e);
  } : undefined;

  return (
    <div class={classes}>
      <label class="file-label">
        <input
          class="file-input"
          type="file"
          onchange={props.onChange}
          multiple={props.multiple}
          accept={
            props.accept && props.accept.length > 0
              ? props.accept.join(",")
              : undefined
          }
          onChange={onChange}
        />
        <span class="file-cta">
          <span class="file-icon">
            <i class="fas fa-upload"></i>
          </span>
          <span class="file-label">{label}</span>
        </span>
        {props.fileName ? (
          <span class="file-name">{props.fileName}</span>
        ) : null}
      </label>
    </div>
  );
}
