import { merge } from 'lodash';
import { JSX } from "solid-js";
import {
  AttributeSchema,
  Cardinality,
  EntitySchema,
  Value,
  ValueType,
} from "../../semantic/core";
import { UiRegistry, ValueMap } from "../../semantic/registry";
import { createForm, FieldAccessor, FormState, FormValidation, FormValidator } from "../form";
import { CheckboxField } from "../form/CheckboxField";
import { FormFooter } from "../form/FormFooter";
import { InputField } from "../form/InputField";
import { SelectOption } from "../form/Select";
import { SelectField } from "../form/SelectField";

function unionPlainOptions(variants: ValueType[]): SelectOption<Value>[] {
  return variants.map((variant) => {
    if (typeof variant === "object" && "Const" in variant && variant["Const"]) {
      return {
        value: variant["Const"],
        label: variant["Const"].toString(),
      };
    } else {
      throw new Error(
        "Unsupported union type - only unions of constant values are supported " +
        JSON.stringify(variant)
      );
    }
  });
}

export function entityAttributeFormField(
  form: FormState<ValueMap>,
  attr: AttributeSchema,
  cardinality: Cardinality
): [JSX.Element, FormValidator<ValueMap>] {
  const name = attr["factor/title"] || attr["factor/ident"];
  const ty = attr["factor/valueType"];

  const attrIdent = attr["factor/ident"];

  if (cardinality === "Many") {
    throw new Error("Many cardinality not supported");
  }

  console.trace({ attr, cardinality })

  if (ty === "Bool") {
    let field = form.field(attrIdent) as FieldAccessor<boolean>;
    return [<CheckboxField label={name} field={field} />, (_values) => null];
  } else if (ty === "String") {
    let field = form.field(attrIdent) as FieldAccessor<string>;
    return [
      <InputField label={name} field={field} />,
      (values) => {
        if (cardinality === "Required") {
          if (!(values[attrIdent] || '').trim()) {
            return {
              fields: {
                [attrIdent]: {
                  errors: [
                    {
                      message: "Required",
                    },
                  ],
                },
              },
            };
          }
        }
        return null;
      },
    ];
  } else if (typeof ty === "object") {
    if ("Union" in ty) {
      const options: SelectOption<Value | undefined>[] = [{ label: '', value: undefined }, ...unionPlainOptions(ty["Union"])];
      let field = form.field(attrIdent) as FieldAccessor<Value>;
      console.error('select field')

      return [
        <SelectField<Value | undefined> label={name} field={field} options={options} />,
        (values) => {
          if (cardinality === "Required") {
            if (!values?.[attrIdent]?.trim()) {
              return {
                fields: {
                  [attrIdent]: {
                    errors: [
                      {
                        message: "Required",
                      },
                    ],
                  },
                },
              };
            }
          }
          return null;
        },
      ];
    }
  }

  throw new Error("unsupported attribute type: " + JSON.stringify(ty));
}

export interface GenericEntityFormProps {
  registry: UiRegistry;
  schema: EntitySchema;

  submitLabel: JSX.Element;
  cancelLabel?: JSX.Element;
  onCancel?: () => void;

  onSubmit?: (values: ValueMap, form: FormState<ValueMap>) => (Promise<void> | void);
}

export function GenericEntityForm(props: GenericEntityFormProps): JSX.Element {
  const form = createForm<Record<string, any>>({
    initialValues: {},
    onSubmit: props.onSubmit,
  });

  const elems: JSX.Element[] = [];
  const validators: FormValidator<ValueMap>[] = [];

  const attrs = props.registry.entityAttributes(props.schema["factor/ident"]);
  for (const [attr, cardinality] of attrs) {
    const [elem, validator] = entityAttributeFormField(form, attr, cardinality);
    elems.push(elem);
    validators.push(validator);
  }

  form.setValidator((values) => {
    let all: FormValidation<ValueMap> = {
      fields: {},
    };
    for (const val of validators) {
      const fieldVal = val(values);
      all = merge(all, fieldVal);
    }
    return all;
  });

  return (
    <form
      onsubmit={(e) => {
        form.handleSubmit(e);
      }}
    >
      {elems}

      <FormFooter
        form={form}
        buttons={{
          submit: { children: props.submitLabel, color: "is-info" },
          onCancel: props.onCancel,
        }}
      />
    </form>
  );
}
