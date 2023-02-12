import { JSX } from "solid-js/jsx-runtime";
export type FileInputSize = "is-small" | "is-normal" | "is-medium" | "is-large";
export interface FileInputProps {
    label?: JSX.Element;
    boxed?: boolean;
    fileName?: string;
    fileNameIsLeft?: boolean;
    fullWidth?: boolean;
    size?: FileInputSize;
    multiple?: boolean;
    accept?: string[];
    onChange?: JSX.EventHandler<HTMLInputElement, Event>;
    onFilesChange?: (files: FileList | null) => void;
}
export declare function FileInput(props: FileInputProps): JSX.Element;
