import { JSX } from "solid-js";
import { Control } from "../bulma/form";
import { Icon } from "./icon";

export interface SearchInputProps {
  value?: string;
  placeholder?: string;
  onChange?: JSX.EventHandlerUnion<HTMLInputElement, Event>;
  onInput?: JSX.EventHandlerUnion<HTMLInputElement, Event>;
  ref?: HTMLInputElement;
}

export function SearchInput(props: SearchInputProps): JSX.Element {
  return (
    <Control class="has-icons-left">
      <input
        value={props.value ?? ""}
        oninput={props.onInput}
        ref={props.ref}
        class="input"
        type="text"
        placeholder={props.placeholder ?? "Search..."}
      />
      <Icon icon="search" isLeft />
    </Control>
  );
}
