export function FileInput(props) {
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
    const onChange = (props.onChange || props.onFilesChange) ? (e) => {
        props.onFilesChange?.(e.currentTarget.files);
        props.onChange?.(e);
    } : undefined;
    return (<div class={classes}>
      <label class="file-label">
        <input class="file-input" type="file" onchange={props.onChange} multiple={props.multiple} accept={props.accept && props.accept.length > 0
            ? props.accept.join(",")
            : undefined} onChange={onChange}/>
        <span class="file-cta">
          <span class="file-icon">
            <i class="fas fa-upload"></i>
          </span>
          <span class="file-label">{label}</span>
        </span>
        {props.fileName ? (<span class="file-name">{props.fileName}</span>) : null}
      </label>
    </div>);
}
