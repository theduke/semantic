import { merge } from "lodash";
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
  value: T | undefined;
  touched: boolean;
  changed: boolean;
  validation?: ValidationResult;
}

export type FormValidator<Values> = (
  values: Values
) => FormValidation<Values> | Promise<FormValidation<Values>> | null;

export interface FormInit<Values extends Record<string, any>> {
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
    validations?: ValidationResult,
    skipValidations: boolean = false
  ) {
    const old = this.state.fields?.[name] as any;

    const isValid = validations?.errors?.length === 0 ?? true;

    const newData: FieldState<Values[K]> = {
      value,
      touched: true,
      changed: old?.changed === true || value !== old?.value,
      validation: validations,
    };

    this.setStore("fields", name as any, newData as any);

    if (!isValid) {
      this.setStore("isValid", false);
    }

    if (!skipValidations && this.init.validateOnChange) {
      this.runValidations();
    } else {
      this.onChanged();
    }
  }

  field<K extends keyof Values>(field: K): FieldAccessor<Values[K]> {
    return new BasicFieldAccessor(this, field as any);
  }

  fieldGetter<K extends keyof Values>(name: K): () => FieldState<Values[K]> {
    return () => (this.state as any).fields[name];
  }

  private buildValues(): Values {
    const values: Values = {} as any;
    const validations: FieldValidations<Values> = {};
    for (const [key, field] of Object.entries(this.state.fields)) {
      if (field) {
        values[key as any as keyof Values] = field.value;
        // TODO: prevent any cast
        validations[key as any as keyof Values] = field.validation;
      } else {
        values[key as any as keyof Values] = this.init.initialValues[key];
      }
    }
    return values;
  }

  private onChanged() {
    if (this.init.onValid) {
      this.init.onValid(this.buildValues());
    }
  }

  private runValidations(): Promise<void> | void {
    if (this.state.isValidating || this.state.isSubmitting) {
      return;
    }

    if (this.init.validate) {
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
      this.onChanged();
      return;
    } else {
      this.applyValidations({});
    }
  }

  private applyValidations(res: FormValidation<Values> | null) {
    const formValid = res?.form?.errors?.length === 0 || true;
    const fieldStateInvalid = Object.values(this.state.fields).find(
      (f) => f.validation?.errors?.length > 0
    );
    const fieldsInvalid = Object.values(res?.fields ?? {}).find(
      (field: ValidationResult) => (field?.errors?.length ?? 0) > 0
    );
    const isValid = fieldStateInvalid && formValid && !fieldsInvalid;

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
    if (
      this.state.isValidating ||
      this.state.isSubmitting ||
      this.state.isValid === false
    ) {
      return;
    }
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

export interface FieldAccessor<T> {
  get(): FieldState<T>;
  errors(): ValidationResult | undefined;
  set(value: T, validations?: ValidationResult): void;
}

class BasicFieldAccessor<T> implements FieldAccessor<T> {
  protected form: FormState<any>;
  protected key: string;

  constructor(form: FormState<any>, key: string) {
    this.form = form;
    this.key = key;
  }

  get(): FieldState<T> {
    return (this.form.state as any).fields[this.key];
  }

  errors(): ValidationResult | undefined {
    const fieldVal = this.get()?.validation;
    const formVal = (this.form.state as any).validation?.fields?.[this.key];
    let errors = undefined;
    if (fieldVal || formVal) {
      errors = merge(fieldVal ?? {}, formVal ?? {});
    }
    return errors;
  }

  set(value: T, validations?: ValidationResult) {
    this.form.setField(this.key, value, validations);
  }
}

export class MappedFieldAccessor<T, M> implements FieldAccessor<M> {
  field: FieldAccessor<T>;
  parse: (value: M) => T;
  format: (value: T) => M;

  constructor(
    field: FieldAccessor<T>,
    parse: (value: M) => T,
    format: (value: T) => M
  ) {
    this.field = field;
    this.parse = parse;
    this.format = format;
  }

  get(): FieldState<M> {
    const state: FieldState<T> = this.field.get();
    const format = this.format;
    return {
      get value(): M {
        return state.value === undefined
          ? undefined
          : (format(state.value) as any);
      },
      changed: state.changed,
      touched: state.touched,
    };
  }

  errors(): ValidationResult | undefined {
    return this.field.errors();
  }

  set(value: M, validations?: ValidationResult) {
    try {
      const parsed = this.parse(value);
      this.field.set(parsed, validations);
    } catch (err: any) {
      this.field.set(undefined as any, {
        errors: [{ message: err.toString() }],
      });
    }
  }
}

export function createForm<Values extends Record<string, any>>(
  init: FormInit<Values>
): FormState<Values> {
  return new FormState(init);
}
