import { JSX, splitProps } from "solid-js";
import { FormState } from ".";
import { Button, ButtonProps } from "../bulma/button";

export interface FormSubmitButtonProps
  extends Omit<ButtonProps, "onclick" | "form"> {
  children: JSX.Element;
  form: FormState<any>;
}

export function FormSubmitButton(props: FormSubmitButtonProps): JSX.Element {
  const [_local, btnProps] = splitProps(props, ["form"]);
  const form = props.form;

  return (
    <Button
      {...btnProps}
      disabled={form.state.isValid}
      loading={form.state.isValidating || form.state.isSubmitting}
      type="submit"
    />
  );
}
