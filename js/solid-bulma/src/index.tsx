export type BulmaColor =
  | "is-primary"
  | "is-link"
  | "is-info"
  | "is-success"
  | "is-warning"
  | "is-danger"
  | "is-black"
  | "is-dark"
  | "is-light"
  | "is-white";

export type BulmaTextColor =
  | "has-text-primary"
  | "has-text-link"
  | "has-text-info"
  | "has-text-success"
  | "has-text-warning"
  | "has-text-danger";

export { Box } from "./box";

import {
  TagSize as OriginalTagSize,
  TagProps as OriginalTagProps,
  Tag,
  DeletableTagProps as OriginalDeletableTagProps,
  DeletableTag,
  TagsSize as OriginalTagsSize,
  TagsProps as OriginalTagsProps,
  Tags,
} from "./tag";
export type TagSize = OriginalTagSize;
export type TagProps = OriginalTagProps;
export type DeletableTagProps = OriginalDeletableTagProps;
export type TagsSize = OriginalTagsSize;
export type TagsProps = OriginalTagsProps;
export { Tag, DeletableTag, Tags };

import {FileInputProps as OriginalFileInputProps, FileInput} from './form/FileInput';
export type FileInputProps = OriginalFileInputProps;
export {FileInput};
