import { JSX, ParentProps } from "solid-js";

export function Field(props: JSX.HTMLAttributes<HTMLDivElement>): JSX.Element {
  let cls = "field";
  if (props.class) {
    cls += " " + props.class;
  }
  return <div {...props} class={cls} />;
}

export interface FieldHorizontalProps extends ParentProps {
  label: JSX.Element;
  smallLabel?: boolean;
}

export function FieldHorizontal(props: FieldHorizontalProps): JSX.Element {
  return (
    <div class="field is-horizontal">
      {props.smallLabel ? (
        <div class="field-label is-normal has-text-left is-flex-grow-0">
          <label class="label">{props.label}</label>
        </div>
      ) : (
        <div class="field-label is-normal">
          <label class="label">{props.label}</label>
        </div>
      )}

      <div class="field-body">
        <div class="field">{props.children}</div>
      </div>
    </div>
  );
}

export function Label(props: ParentProps): JSX.Element {
  return <label class="label">{props.children}</label>;
}

export interface ControlProps extends JSX.HTMLAttributes<HTMLDivElement> {}

export function Control(props: ControlProps): JSX.Element {
  let cls = "control";
  if (props.class) {
    cls += " " + props.class;
  }

  return <div {...props} class={cls} />;
}
