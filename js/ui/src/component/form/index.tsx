import { batch, createEffect } from "solid-js";
import {
  createStore,
  produce,
  reconcile,
  SetStoreFunction,
  Store,
} from "solid-js/store";
import { isPromise } from "..";

export type FormValidationType = "error" | "warning";

export interface FormError {
  message: string;
  details?: string;
}

export interface ValidationResult {
  errors?: FormError[];
  warnings?: FormError[];
}

type MappedValues<Values, T> = {
  [K in keyof Values]?: Values[K] extends any[]
    ? Values[K][number] extends object // [number] is the special sauce to get the type of array's element. More here https://github.com/Microsoft/TypeScript/pull/21316
      ? MappedValues<Values[K][number], T>[]
      : T[]
    : Values[K] extends object
    ? MappedValues<Values[K], T>
    : T;
};

export type FieldValidations<Values> = MappedValues<Values, ValidationResult>;

export type FormValidation<Values> = {
  fields?: FieldValidations<Values>;
  form?: ValidationResult;
};

export const VALID = "valid";

interface FieldState<T> {
  value: T;
  touched: boolean;
  changed: boolean;
}

export type FormValidator<Values> = (
  values: Values
) => FormValidation<Values> | Promise<FormValidation<Values>> | null;

export interface FormInit<Values> {
  initialValues: Values;
  validate?: FormValidator<Values>;
  validateOnMount?: boolean;
  validateOnChange?: boolean;
  onSubmit?: (values: Values, form: FormState<Values>) => Promise<void> | void;
  onValid?: (values: Values) => void;
}

type StoreFields<Values> = {
  [K in keyof Values]?: Values[K] extends any[]
    ? Values[K][number] extends object // [number] is the special sauce to get the type of array's element. More here https://github.com/Microsoft/TypeScript/pull/21316
      ? StoreFields<Values[K][number]>[]
      : FieldState<[Values[K]]>
    : Values[K] extends object
    ? StoreFields<Values[K]>
    : FieldState<Values[K]>;
};

type StoreData<Values> = {
  fields: StoreFields<Values>;
  isValidating: boolean;
  validation?: FormValidation<Values>;
  isValid?: boolean;
  isSubmitting: boolean;
};

export class FormState<Values extends Record<string, any>> {
  // private onSubmit?: (value: F) => FormSubmitResult;
  // private onSubmitAsync?: (value: F) => Promise<FormSubmitResult>;
  // private builder?: (values: {[key in keyof F]: any}) => T;

  init: FormInit<Values>;
  private setStore: SetStoreFunction<StoreData<Values>>;
  state: Store<StoreData<Values>>;

  constructor(init: FormInit<Values>) {
    if (init.validateOnChange === undefined) {
      init.validateOnChange = true;
    }
    // this.onSubmit = init.onSubmit;
    // this.onSubmitAsync = init.onSubmitAsync;

    this.setField = this.setField.bind(this);
    this.handleSubmit = this.handleSubmit.bind(this);

    this.init = init;

    const fields: StoreFields<Values> = {};
    for (const [key, value] of Object.entries(init.initialValues)) {
      (fields as any)[key as any] = {
        value,
        touched: false,
        changed: false,
      };
    }

    const [store, setStore] = createStore<StoreData<Values>>({
      fields,
      isSubmitting: false,
      validation: null,
    } as any);

    this.state = store;
    this.setStore = setStore;

    if (init.validateOnMount) {
      createEffect(() => {
        this.runValidations();
      });
    }
  }

  setValidator(val: FormValidator<Values>) {
    this.init.validate = val;
  }

  reset() {
    batch(() => {
      this.setStore("fields", reconcile({} as any, { merge: false }));
      this.setStore("validation", undefined);
    });
  }

  setField<K extends keyof Values>(
    name: K,
    value: Values[K],
    skipValidations: boolean = false
  ) {
    const old = this.state.fields?.[name] as any;
    this.setStore(
      "fields",
      name as any,
      {
        value,
        touched: true,
        changed: old?.changed ?? value !== old?.value,
      } as any
    );
    if (!skipValidations && this.init.validateOnChange) {
      this.runValidations();
    } else {
      this.onChanged();
    }
  }

  field<K extends keyof Values>(field: K): FieldAccessor<Values[K]> {
    return new FieldAccessor(this, field as any);
  }

  fieldGetter<K extends keyof Values>(name: K): () => FieldState<Values[K]> {
    return () => (this.state as any).fields[name];
  }

  buildValues(): Values {
    const values = {} as any;
    for (const [key, field] of Object.entries(this.state.fields)) {
      const value =
        field === undefined ? this.init.initialValues[key] : field.value;
      values[key] = value;
    }
    return values;
  }

  onChanged() {
    if (this.init.onValid) {
      this.init.onValid(this.buildValues());
    }
  }

  runValidations(): Promise<void> | void {
    if (!this.init.validate) {
      return;
    }
    if (this.state.isValidating || this.state.isSubmitting) {
      return;
    }

    try {
      const res = this.init.validate(this.buildValues());

      if (isPromise(res)) {
        this.setStore("isValidating", true);
        return res
          .then((res) => {
            this.applyValidations(res);
          })
          .catch((error) => {
            console.error("Form validator threw an exception", { error });
            this.applyValidations({
              form: { errors: [{ message: (error as any)?.toString() }] },
            });
          });
      } else {
        this.applyValidations(res);
      }
    } catch (error) {
      console.error("Form validator threw an exception", { error });
      this.applyValidations({
        form: { errors: [{ message: (error as any)?.toString() }] },
      });
    }
  }

  private applyValidations(res: FormValidation<Values> | null) {
    const formValid = res?.form?.errors?.length === 0 || true;
    const fieldsInvalid = Object.values(res?.fields ?? {}).find(
      (field: ValidationResult) => (field?.errors?.length ?? 0) > 0
    );
    const isValid = formValid && !fieldsInvalid;

    this.setStore(
      produce((state) => {
        state.isValidating = false;
        state.isValid = isValid;
        (state.validation as any) = res;
      })
    );

    this.onChanged();
  }

  handleSubmit(e: Event) {
    e.preventDefault();
    e.stopPropagation();

    this.submit();
  }

  submit() {
    if (
      !this.init.onSubmit ||
      this.state.isValidating ||
      this.state.isSubmitting
    ) {
      return;
    }
    const validated = this.runValidations();
    if (isPromise(validated)) {
      validated.then(() => {
        this.invokeSubmit();
      });
    } else {
      this.invokeSubmit();
    }
  }

  private invokeSubmit() {
    try {
      const values = this.buildValues();
      const res = this.init?.onSubmit?.(values, this);
      console.log({ res, isPromise: isPromise(res) });
      if (isPromise(res)) {
        this.setStore("isSubmitting", true);

        res
          .then(() => {
            this.setStore("isSubmitting", false);
          })
          .catch((error) => {
            console.error("Form submit threw an exception", { error });
            this.setStore(
              produce((store) => {
                store.validation = {
                  form: { errors: [{ message: (error as any)?.toString() }] },
                };
                store.isSubmitting = false;
              })
            );
          });
      }
    } catch (error) {
      console.error("Form submit handler threw an exception", { error });
      this.setStore("validation", {
        form: { errors: [{ message: (error as any)?.toString() }] },
      });
    }
  }
}

export class FieldAccessor<T> {
  private form: FormState<any>;
  private key: string;

  constructor(form: FormState<any>, key: string) {
    this.form = form;
    this.key = key;
  }

  get(): FieldState<T> {
    return (this.form.state as any).fields[this.key];
  }

  errors(): ValidationResult | undefined {
    return (this.form.state as any).validation?.fields?.[this.key];
  }

  set(value: T) {
    this.form.setField(this.key, value);
  }
}

export function createForm<Values>(init: FormInit<Values>): FormState<Values> {
  return new FormState(init);
}
