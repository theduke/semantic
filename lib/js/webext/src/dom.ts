export function requiredElement<T extends HTMLElement>(
  id: string,
  constructor: { new (): T },
): T {
  const element = document.getElementById(id);
  if (!(element instanceof constructor)) {
    throw new Error(`Missing required element #${id}`);
  }
  return element;
}

export function setStatus(
  element: HTMLElement,
  message: string,
  kind: "idle" | "busy" | "success" | "error" = "idle",
): void {
  element.textContent = message;
  element.dataset.kind = kind;
}
