import { JSX } from "solid-js/jsx-runtime";
import { FormState } from ".";
import { FormButtons, FormButtonsProps } from "./FormButtons";
import { FormErrors } from "./FormErrors";

export interface FormFooterProps {
  buttons?: Omit<FormButtonsProps, "form">;
  form: FormState<any>;
}

export function FormFooter(props: FormFooterProps): JSX.Element {
  return (
    <div>
      <FormErrors form={props.form} />
      <FormButtons {...props.buttons} form={props.form} />
    </div>
  );
}
