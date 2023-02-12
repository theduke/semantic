import { JSX, ParentProps } from "solid-js";
import { BulmaColor } from "./";
export type TagSize = "is-normal" | "is-medium" | "is-large";
export interface TagProps extends JSX.HTMLAttributes<HTMLSpanElement> {
    color?: BulmaColor;
    size?: TagSize;
    isLight?: boolean;
    rounded?: boolean;
}
export declare function Tag(props: TagProps): JSX.Element;
export interface DeletableTagProps extends TagProps {
    onDelete: () => void;
    wholeTagDelete?: boolean;
}
export declare function DeletableTag(props: DeletableTagProps): JSX.Element;
export type TagsSize = "are-medium" | "are-large";
export interface TagsProps extends ParentProps {
    attached?: boolean;
    size?: TagsSize;
}
export declare function Tags(props: TagsProps): JSX.Element;
