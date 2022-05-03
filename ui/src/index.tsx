/* @refresh reload */
import { render } from "solid-js/web";

import "bulma/css/bulma.min.css";
import "@fortawesome/fontawesome-free/css/all.css";
import "./index.css";

import { App } from "./App";

// Asserts that a value is defined and returns it, or throws an exception.
//
// Needed for solid js <Show> when we know that a checked value will be valid.
export function assertDefined<T>(value: T | null | undefined): T {
  if (value === null || value === undefined) {
    throw new Error("Expected value to be valid");
  }
  return value;
}

render(() => <App />, document.getElementById("root") as HTMLElement);
