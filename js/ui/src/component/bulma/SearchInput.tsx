import { JSX, onMount } from "solid-js";
import { Control } from "../bulma/form";
import { Icon } from "./icon";

export interface SearchInputProps {
  value?: string;
  placeholder?: string;
  onChange?: JSX.EventHandlerUnion<HTMLInputElement, Event>;
  onInput?: JSX.EventHandlerUnion<HTMLInputElement, Event>;
  autoFocus?: boolean;
}

export function SearchInput(props: SearchInputProps): JSX.Element {
  let inputRef: HTMLInputElement | undefined;

  // Auto-focus input element if enabled.
  if (props.autoFocus) {
    onMount(() => {
      inputRef?.focus();
    });
  }

  return (
    <Control class="has-icons-left">
      <input
        style={{ width: "100%" }}
        ref={inputRef}
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
