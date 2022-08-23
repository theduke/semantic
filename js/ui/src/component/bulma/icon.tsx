import { JSX } from "solid-js";
import { TextColor } from ".";

const FA_ICON_MAP = {
  train: "fas fa-train",
  userGear: "fa-solid fa-user-gear",
  signOut: "fa-solid fa-right-from-bracket",
  search: "fas fa-search",
  pencil: "fa-solid fa-pencil",
  table: "fa-solid fa-table",
  plus: "fa-solid fa-plus",
  minus: "fa-solid fa-minus",
  arrowsCross: "fa-solid fa-up-down-left-right",
  trash: "fa-solid fa-trash",
  xmark: "fa-solid fa-xmark",
  circleXmarkRegular: "fa-regular fa-circle-xmark",
  filter: "fa-solid fa-filter",
  globe: "fa-solid fa-globe",
  upload: "fa-solid fa-upload",
  play: "fa-solid fa-play",
  pause: "fa-solid fa-play",
  caretRight: "fa-solid fa-caret-right",
  arrowLeft: "fa-solid fa-arrow-left",
  arrowRight: "fa-solid fa-arrow-right",
  caretLeft: "fa-solid fa-caret-left",
  shuffle: "fa-solid fa-shuffle",
  rotateLeft: "fa-solid fa-rotate-left",
  volumeXmark: "fa-solid fa-volume-xmark",
  volumeHigh: "fa-solid fa-volume-high",
  arrowsRotate: "fa-solid fa-arrows-rotate",
  angleLeft: "fa-solid fa-angle-left",
  angleRight: "fa-solid fa-angle-right",
  arrowsLeftRight: "fa-solid fa-arrows-left-right",
  tags: "fa-solid fa-tags",
};

export type IconSize = "is-small" | "is-medium" | "is-large";

export type IconName = keyof typeof FA_ICON_MAP;

export function fontawesomeIconClass(name: IconName): string {
  const cls = FA_ICON_MAP[name];
  if (!cls) {
    console.error("Used unknown icon name in <Icon> component!", name);
  }
  return cls;
}

export interface IconProps {
  icon: IconName;
  color?: TextColor;
  size?: IconSize;
  isLeft?: boolean;
}

export function Icon(props: IconProps): JSX.Element {
  let cls = "icon";
  if (props.color) {
    cls += " " + props.color;
  }
  if (props.size) {
    cls += " " + props.size;
  }
  if (props.isLeft) {
    cls += " is-left";
  }
  return (
    <span aria-hidden class={cls}>
      <i class={fontawesomeIconClass(props.icon)} />
    </span>
  );
}

export interface IconTextProps {
  icon: IconName;
  iconColor?: TextColor;
  color?: TextColor;
  textColor?: TextColor;
  text: string;
  // By default the elemnt is inline-flex. If notInline is set, the icon will
  // be a flex element.
  notInline?: boolean;
}

export function IconText(props: IconTextProps): JSX.Element {
  let cls = "icon-text";
  if (props.color) {
    cls += " " + props.color;
  }

  const inner = (
    <>
      <Icon icon={props.icon} color={props.color} />
      <span class={props.textColor}>{props.text}</span>
    </>
  );

  if (props.notInline) {
    return <div class={cls}>{inner}</div>;
  } else {
    return <span class={cls}>{inner}</span>;
  }
}
