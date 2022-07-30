import { splitProps } from "solid-js";
export function Tag(props) {
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
    return <span class={cls}>{props.children}</span>;
}
// A tag with a small delete icon.
export function DeletableTag(props) {
    const [_, rest] = splitProps(props, ['onDelete']);
    return (<Tag {...rest}>
      {props.children}
      <button class="delete" onclick={props.onDelete}/>
    </Tag>);
}
export function Tags(props) {
    let cls = "tags";
    if (props.attached) {
        cls += " has-addons";
    }
    if (props.size) {
        cls += " " + props.size;
    }
    return <div class={cls}>{props.children}</div>;
}
