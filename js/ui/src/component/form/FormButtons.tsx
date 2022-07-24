import { JSX } from "solid-js";
import { FormState } from ".";
import { Button, ButtonProps, Buttons } from "../bulma/button";
import { FormSubmitButton, FormSubmitButtonProps } from "./FormSubmitButton";

export interface FormButtonsProps {
  form: FormState<any>;
  submit?: Omit<FormSubmitButtonProps, "form">;
  cancelButton?: ButtonProps;
  onCancel?: () => void;
}

export function FormButtons(props: FormButtonsProps): JSX.Element {
  console.log({ cancelProps: props.cancelButton, onCancel: props.onCancel });
  const cancel = props.onCancel ? (
    <Button
      {...(props.cancelButton ?? {})}
      children={props?.cancelButton?.children ?? "Cancel"}
      onclick={props.onCancel}
    />
  ) : null;
  return (
    <Buttons>
      <FormSubmitButton
        {...props.submit}
        form={props.form}
        children={props?.submit?.children ?? "Submit"}
      />
      {cancel}
    </Buttons>
  );
}
