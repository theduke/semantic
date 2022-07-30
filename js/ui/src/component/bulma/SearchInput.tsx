import { JSX, onMount } from "solid-js";
import { Control } from "../bulma/form";
import { Icon } from "./icon";

export interface SearchInputProps {
  value?: string;
  placeholder?: string;
  onChange?: JSX.EventHandlerUnion<HTMLInputElement, Event>;
  onInput?: JSX.EventHandlerUnion<HTMLInputElement, Event>;
}

export function SearchInput(props: SearchInputProps): JSX.Element {

  let ref: HTMLDivElement | undefined;

  onMount(() => {
    const obs = new MutationObserver((mutations) => {
      console.debug({ mutations });
      debugger;
    });

    if (ref) {
      obs.observe(ref, {childList: true});
    }
  })

  return (
    <Control class="has-icons-left" ref={ref}>
      <input
        value={props.value ?? ""}
        oninput={props.onInput}
        onchange={props.onChange}
        class="input"
        type="text"
        placeholder={props.placeholder ?? "Search..."}
      />
      <Icon icon="search" isLeft />
    </Control>
  );
}
