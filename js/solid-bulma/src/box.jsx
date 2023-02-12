export function Box(props) {
    const cls = "box" + (props.class ? " " + props.class : "");
    return <div {...props} class={cls}/>;
}
