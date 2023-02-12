import { splitProps } from "solid-js";
export function Tag(props) {
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
// A tag with a small delete icon.
export function DeletableTag(props) {
    const [_, rest] = splitProps(props, ["onDelete"]);
    if (props.wholeTagDelete) {
        <Tag style={{ cursor: 'pointer' }} onclick={props.onDelete} {...rest}>
        {props.children}
        <button class="delete" onclick={props.onDelete}/>
      </Tag>;
    }
    else {
        return (<Tag {...rest}>
        {props.children}
        <button class="delete" onclick={props.onDelete}/>
      </Tag>);
    }
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
