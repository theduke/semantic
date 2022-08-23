import { JSX, ParentProps, splitProps } from "solid-js";
import { BulmaColor } from "./";

export type TagSize = "is-normal" | "is-medium" | "is-large";

export interface TagProps extends JSX.HTMLAttributes<HTMLSpanElement> {
  color?: BulmaColor;
  size?: TagSize;
  isLight?: boolean;
  rounded?: boolean;
}

export function Tag(props: TagProps): JSX.Element {
  const [local, rest] = splitProps(props, ["color", "size", "isLight", "rounded"]);

  let cls = "tag";

  if (props.color) {
    cls += " " + props.color;
  }
  if (props.size) {
    cls += " " + props.size;
  }
  if (props.isLight) {
    cls += " is-light";
  }
  if (props.rounded) {
    cls += " is-rounded";
  }

  return <span class={cls} {...rest}>{props.children}</span>;
}

export interface DeletableTagProps extends TagProps {
  onDelete: () => void;
  wholeTagDelete?: boolean;
}

// A tag with a small delete icon.
export function DeletableTag(props: DeletableTagProps): JSX.Element {
  const [_, rest] = splitProps(props, ["onDelete"]);
  if (props.wholeTagDelete) {
      <Tag style={{cursor: 'pointer'}} onclick={props.onDelete} {...rest}>
        {props.children}
        <button class="delete" onclick={props.onDelete} />
      </Tag>
  } else {
    return (
      <Tag {...rest}>
        {props.children}
        <button class="delete" onclick={props.onDelete} />
      </Tag>
    );
  }

}

export type TagsSize = "are-medium" | "are-large";

export interface TagsProps extends ParentProps {
  attached?: boolean;
  size?: TagsSize;
}

export function Tags(props: TagsProps): JSX.Element {
  let cls = "tags";
  if (props.attached) {
    cls += " has-addons";
  }
  if (props.size) {
    cls += " " + props.size;
  }
  return <div class={cls}>{props.children}</div>;
}
