import {
  selectLabel,
  type MetadataKind,
  type MetadataOption,
} from "./metadata.js";

type Search = (kind: MetadataKind, query: string) => Promise<MetadataOption[]>;
const names = {
  directory: {
    title: "Folder",
    action: "Set folder",
    placeholder: "Search folders…",
    empty: "No folders found.",
  },
  parent: {
    title: "Parent entity",
    action: "Set parent",
    placeholder: "Search entities…",
    empty: "No entities found.",
  },
  labels: {
    title: "Labels",
    action: "Add labels",
    placeholder: "Search labels…",
    empty: "No labels found. Create labels in Semantic.",
  },
};

/** An inline picker with native keyboard controls and removable selections. */
export class MetadataPicker {
  selected: MetadataOption[] = [];
  private readonly trigger = document.createElement("button");
  private readonly chips = document.createElement("div");
  private readonly editor = document.createElement("div");
  private readonly input = document.createElement("input");
  private readonly results = document.createElement("div");
  private readonly notice = document.createElement("p");
  private generation = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private disabled = false;

  constructor(
    private readonly root: HTMLElement,
    readonly kind: MetadataKind,
    private readonly search: Search,
    private readonly onOpen: () => void,
  ) {
    const copy = names[kind];
    root.className = "metadata-row";
    const heading = document.createElement("span");
    heading.className = "metadata-label";
    heading.textContent = copy.title;
    this.chips.className = "metadata-values";
    this.trigger.type = "button";
    this.trigger.className = "metadata-add";
    this.trigger.textContent = `+ ${copy.action}`;
    this.trigger.setAttribute("aria-expanded", "false");
    this.trigger.setAttribute("aria-controls", `${kind}-editor`);
    this.editor.id = `${kind}-editor`;
    this.editor.className = "metadata-editor";
    this.editor.hidden = true;
    const searchRow = document.createElement("div");
    searchRow.className = "metadata-search";
    this.input.type = "search";
    this.input.placeholder = copy.placeholder;
    this.input.setAttribute("aria-label", copy.placeholder.replace("…", ""));
    this.input.autocomplete = "off";
    const close = document.createElement("button");
    close.type = "button";
    close.className = "picker-done";
    close.textContent = "Done";
    close.addEventListener("click", () => this.close(true));
    searchRow.append(this.input, close);
    this.notice.className = "picker-notice";
    this.notice.setAttribute("role", "status");
    this.results.className = "metadata-results";
    this.results.setAttribute("role", "group");
    this.results.setAttribute("aria-label", `${copy.title} results`);
    this.editor.append(searchRow, this.notice, this.results);
    const controls = document.createElement("div");
    controls.className = "metadata-controls";
    controls.append(this.chips, this.trigger);
    root.append(heading, controls, this.editor);
    this.trigger.addEventListener("click", () => {
      if (!this.editor.hidden) return this.close(true);
      this.onOpen();
      this.editor.hidden = false;
      this.trigger.setAttribute("aria-expanded", "true");
      this.input.value = "";
      this.input.focus();
      void this.load();
    });
    this.input.addEventListener("input", () => {
      clearTimeout(this.timer);
      this.generation++;
      this.results.replaceChildren();
      this.notice.textContent = "Searching…";
      this.timer = setTimeout(() => void this.load(), 180);
    });
    this.editor.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        this.close(true);
      }
      if (event.key === "Enter" && event.target === this.input)
        event.preventDefault();
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        const buttons = Array.from(this.results.querySelectorAll("button"));
        const index = buttons.indexOf(
          document.activeElement as HTMLButtonElement,
        );
        const next =
          event.key === "ArrowDown" ? buttons[index + 1] : buttons[index - 1];
        if (next) {
          event.preventDefault();
          next.focus();
        } else if (event.key === "ArrowUp" && index === 0) {
          event.preventDefault();
          this.input.focus();
        }
      }
    });
  }

  close(focus = false): void {
    clearTimeout(this.timer);
    this.generation++;
    this.editor.hidden = true;
    this.trigger.setAttribute("aria-expanded", "false");
    if (focus) this.trigger.focus();
  }

  setDisabled(disabled: boolean): void {
    this.disabled = disabled;
    if (disabled) this.close();
    this.root.querySelectorAll("button, input").forEach((element) => {
      (element as HTMLButtonElement | HTMLInputElement).disabled = disabled;
    });
  }

  private renderSelection(): void {
    this.chips.replaceChildren();
    for (const option of this.selected) {
      const chip = document.createElement("span");
      chip.className = "metadata-chip";
      chip.title = [option.detail, option.title].filter(Boolean).join(" / ");
      if (option.color) chip.append(colorDot(option.color));
      const title = document.createElement("span");
      title.textContent = option.title;
      const remove = document.createElement("button");
      remove.type = "button";
      remove.className = "chip-remove";
      remove.textContent = "×";
      remove.setAttribute(
        "aria-label",
        `Remove ${names[this.kind].title.toLowerCase()}: ${option.title}`,
      );
      remove.addEventListener("click", () => {
        this.selected = this.selected.filter((item) => item.id !== option.id);
        this.renderSelection();
        if (!this.editor.hidden) void this.load();
        this.trigger.focus();
      });
      chip.append(title, remove);
      this.chips.append(chip);
    }
    this.trigger.textContent =
      this.selected.length && this.kind !== "labels"
        ? "Change"
        : `+ ${names[this.kind].action}`;
  }

  private async load(): Promise<void> {
    const generation = ++this.generation;
    this.results.replaceChildren();
    this.notice.textContent = "Searching…";
    try {
      const options = await this.search(this.kind, this.input.value);
      if (generation !== this.generation || this.disabled) return;
      this.notice.textContent = options.length
        ? this.kind === "labels"
          ? "Select labels, then choose Done."
          : "Choose a result below."
        : names[this.kind].empty;
      for (const option of options) {
        const button = document.createElement("button");
        button.type = "button";
        button.className = "metadata-option";
        const selected = this.selected.some((item) => item.id === option.id);
        button.setAttribute("aria-pressed", String(selected));
        if (option.color) button.append(colorDot(option.color));
        const text = document.createElement("span");
        const title = document.createElement("strong");
        title.textContent = option.title;
        const detail = document.createElement("small");
        detail.textContent = option.detail;
        text.append(title, detail);
        const mark = document.createElement("span");
        mark.className = "selection-mark";
        mark.textContent = selected ? "✓" : "+";
        mark.setAttribute("aria-hidden", "true");
        button.append(text, mark);
        button.addEventListener("click", () => {
          this.selected =
            this.kind === "labels"
              ? this.selected.some((item) => item.id === option.id)
                ? this.selected.filter((item) => item.id !== option.id)
                : selectLabel(this.selected, option)
              : [option];
          this.renderSelection();
          if (this.kind === "labels") {
            this.editor.scrollIntoView({ block: "nearest" });
            // Keep the result list and keyboard focus stable during multi-selection.
            for (const [index, child] of Array.from(
              this.results.children,
            ).entries()) {
              const active = this.selected.some(
                (item) => item.id === options[index]?.id,
              );
              child.setAttribute("aria-pressed", String(active));
              child.querySelector(".selection-mark")!.textContent = active
                ? "✓"
                : "+";
            }
          } else this.close(true);
        });
        this.results.append(button);
      }
      this.editor.scrollIntoView({ block: "nearest" });
    } catch (error) {
      if (generation !== this.generation) return;
      this.notice.textContent =
        error instanceof Error ? error.message : "Search failed.";
      const retry = document.createElement("button");
      retry.type = "button";
      retry.className = "picker-done";
      retry.textContent = "Try again";
      retry.addEventListener("click", () => void this.load());
      this.results.append(retry);
    }
  }
}

function colorDot(color: string): HTMLElement {
  const dot = document.createElement("span");
  dot.className = "label-dot";
  dot.style.backgroundColor = color;
  return dot;
}
