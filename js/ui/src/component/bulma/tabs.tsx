import {
  Accessor,
  Component,
  createSignal,
  JSX,
  Setter,
  Signal,
} from "solid-js";
import { Dynamic } from "solid-js/web";
import { ButtonSize } from "./button";

export type TabsAlignment = "is-centered" | "is-right";

export type TabsStyle = "is-boxed" | "is-toggle" | "is-toggle is-rounded";

export type TabIndex = number;

export interface TabsProps {
  align?: TabsAlignment;
  size?: ButtonSize;
  style?: TabsStyle;
  fullWidth?: boolean;
  onChange: (index: TabIndex) => void;
  children: JSX.Element[];
}

export function Tabs(props: TabsProps): JSX.Element {
  const [activeIndex, setActiveIndex] = createSignal<TabIndex>(0);

  let cls = "tabs";
  if (props.size) {
    cls += " " + props.size;
  }
  if (props.fullWidth) {
    cls += " is-fullwidth";
  }
  if (props.align) {
    cls += " " + props.align;
  }
  if (props.style) {
    cls += " " + props.style;
  }

  const onChange = (index: TabIndex) => {
    setActiveIndex(index);
    props.onChange(index);
  };

  return (
    <div class={cls}>
      <ul>
        {props.children.map((child, index) => (
          <li
            classList={{
              "is-active": activeIndex() === index,
            }}
            class={`${index === activeIndex() ? "is-active" : ""}`}
          >
            <a onclick={() => onChange(index)}>{child}</a>
          </li>
        ))}
      </ul>
    </div>
  );
}

export interface TabberItem {
  label: JSX.Element;
  children: Component;
}

export interface TabberProps {
  activeIndex?: Signal<TabIndex>;
  initialIndex?: TabIndex;
  items: TabberItem[];
}

export function Tabber(props: TabberProps): JSX.Element {
  const { items } = props;
  const [activeIndex, setActiveIndex] =
    props.activeIndex ?? createSignal<number>(props.initialIndex ?? 0);

  return (
    <div>
      <Tabs activeIndex={activeIndex} setActiveIndex={setActiveIndex}>
        {items.map((item) => item.label)}
      </Tabs>

      <Dynamic component={items[activeIndex()].children} />
    </div>
  );
}
