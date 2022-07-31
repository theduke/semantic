import { merge } from "lodash";
import { Box } from "solid-bulma";
import { createEffect, createSignal, For, JSX, Show } from "solid-js";
import {
  AttributeSchema,
  Cardinality,
  EntitySchema,
  Expr,
  Id,
  Value,
  ValueType,
} from "semantic/dist/core";
import { exprAttr, exprLiteral, exprNotEq } from "semantic/dist/db";
import { UiRegistry, ValueMap } from "../../semantic/registry";
import {
  FACTOR_ID,
  FACTOR_TYPE,
} from "semantic/dist/schema";
import { Button, Buttons } from "../bulma/button";
import { FieldHorizontal } from "../bulma/form";
import { Icon } from "../bulma/icon";
import {
  createForm,
  FieldAccessor,
  FormState,
  FormValidation,
  FormValidator,
} from "../form";
import { CheckboxField } from "../form/CheckboxField";
import { DateTimeInputField } from "../form/DateTimeInputField";
import { FormFooter } from "../form/FormFooter";
import { InputType } from "../form/Input";
import { InputField } from "../form/InputField";
import { SelectOption } from "../form/Select";
import { SelectField } from "../form/SelectField";
import { SearchSelect } from "../util/SearchSelect";
import { EntityPicker } from "./EntityPicker";

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

function makeStringValidator(
  attr: string,
  required: boolean
): FormValidator<ValueMap> {
  if (required) {
    return (values) => {
      const value = values[attr]?.trim();

      if (!value || value.length < 1) {
        return {
          fields: {
            [attr]: {
              errors: [
                {
                  message: attr + " is required",
                },
              ],
            },
          },
        };
      } else {
        return null;
      }
    };
  } else {
    return (_values) => null;
  }
}

export function entityAttributeFormField(
  form: FormState<ValueMap>,
  attr: AttributeSchema,
  cardinality: Cardinality
): [JSX.Element, FormValidator<ValueMap>] {
  const attributeName = attr["factor/title"] || attr["factor/ident"];
  const ty = attr["factor/valueType"];

  const attrIdent = attr["factor/ident"];

  if (cardinality === "Many") {
    throw new Error("Many cardinality not supported");
  }

  const isRequired = cardinality === "Required";

  if (ty === "Bool") {
    let field = form.field(attrIdent) as FieldAccessor<boolean>;
    return [
      <CheckboxField label={attributeName} field={field} />,
      (_values) => null,
    ];
  } else if (ty === "String" || ty === "Url") {
    let field = form.field(attrIdent) as FieldAccessor<string>;

    let inputType: InputType;
    let val: FormValidator<ValueMap>;
    switch (ty) {
      case "String":
        inputType = "text";
        val = makeStringValidator(attrIdent, isRequired);
        break;

      case "Url":
        inputType = "url";
        // TODO: validate correct URL
        val = makeStringValidator(attrIdent, isRequired);
        break;

      default:
        throw new Error(`Unsupported value type: ${ty}`);
    }

    let elem = (
      <InputField type={inputType} label={attributeName} field={field} />
    );

    return [elem, val];
  } else if (ty === "DateTime") {
    let field = form.field(attrIdent) as FieldAccessor<number>;
    let validator = (values: ValueMap) => {
      const value = values[attrIdent];

      if (isRequired) {
        if (!value) {
          return {
            fields: {
              [attrIdent]: {
                errors: [
                  {
                    message: attributeName + " is required",
                  },
                ],
              },
            },
          };
        }
      }

      return null;
    };

    let elem = <DateTimeInputField label={attributeName} field={field} />;

    return [elem, validator];
  } else if (ty === "Ref") {
    let field = form.field(attrIdent) as FieldAccessor<Id | undefined>;

    const id = form.init.initialValues[FACTOR_ID];

    const baseFilter: Expr | undefined = id
      ? exprNotEq(exprAttr(FACTOR_ID), exprLiteral(id))
      : undefined;

    const picker = (
      <EntityPicker
        baseFilter={baseFilter}
        onSelect={(values) => {
          const id = values[FACTOR_ID];
          if (id) {
            field.set(id);
          }
        }}
      />
    );
    const elem = (
      <FieldHorizontal label={attributeName}>
        <Show when={field.get()?.value} fallback={picker}>
          {(id) => {
            return (
              <div>
                <span class="pr-2">{id}</span>
                <Button size="is-small" onClick={() => field.set(undefined)}>
                  <Icon icon="xmark" />
                </Button>
              </div>
            );
          }}
        </Show>
      </FieldHorizontal>
    );

    return [elem, (_values) => null];
  } else if (typeof ty === "object") {
    if ("Union" in ty) {
      const options: SelectOption<Value | undefined>[] = [
        { label: "", value: undefined },
        ...unionPlainOptions(ty["Union"]),
      ];
      let field = form.field(attrIdent) as FieldAccessor<Value>;

      return [
        <SelectField<Value | undefined>
          label={attributeName}
          field={field}
          options={options}
        />,
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

  throw new Error(
    `unsupported attribute type for attribute ${attrIdent}: ${JSON.stringify(
      ty
    )}`
  );
}

export interface GenericEntityFormProps {
  registry: UiRegistry;
  schema?: EntitySchema | null;

  forbidExtraAttributes?: boolean;

  initialValues?: ValueMap;

  submitLabel: JSX.Element;
  cancelLabel?: JSX.Element;
  onCancel?: () => void;

  onSubmit?: (
    values: ValueMap,
    form: FormState<ValueMap>
  ) => Promise<void> | void;
}

interface ExtraAttrItem {
  schema: AttributeSchema;
  validator: FormValidator<ValueMap>;
  rendered: JSX.Element;
}

export function GenericEntityForm(props: GenericEntityFormProps): JSX.Element {
  const form = createForm<Record<string, any>>({
    initialValues: props.initialValues || {},
    onSubmit: (values, form) => {
      const fullValues = {
        ...values,
      };
      if (props.schema) {
        fullValues["factor/type"] = props.schema["factor/ident"];
      }
      return props.onSubmit?.(fullValues, form);
    },
  });

  const [extraAttrs, setExtraAttrs] = createSignal<ExtraAttrItem[]>([]);

  const elems: JSX.Element[] = [];
  const validators: FormValidator<ValueMap>[] = [];

  const handledAttrs = new Set<string>();
  handledAttrs.add(FACTOR_ID);
  handledAttrs.add(FACTOR_TYPE);
  if (props.schema) {
    const schemaAttrs = props.registry.entityAttributes(
      props.schema["factor/ident"]
    );
    for (const [attr, cardinality] of schemaAttrs) {
      const [elem, validator] = entityAttributeFormField(
        form,
        attr,
        cardinality
      );
      elems.push(elem);
      validators.push(validator);
      handledAttrs.add(attr["factor/ident"]);
    }

    if (props.initialValues) {
      for (const attrIdent of Object.keys(props.initialValues)) {
        if (!handledAttrs.has(attrIdent)) {
          const attrSchema = props.registry.attrs[attrIdent];
          if (attrSchema) {
            const [elem, validator] = entityAttributeFormField(
              form,
              attrSchema,
              "Required"
            );
            elems.push(elem);
            validators.push(validator);
          }
        }
      }
    }
  } else if (props.initialValues) {
    for (const attrIdent of Object.keys(props.initialValues)) {
      if (!handledAttrs.has(attrIdent)) {
        const attrSchema = props.registry.attrs[attrIdent];
        if (attrSchema) {
          const [elem, validator] = entityAttributeFormField(
            form,
            attrSchema,
            "Required"
          );
          elems.push(elem);
          validators.push(validator);
        }
      }
    }
  }

  form.setValidator((values) => {
    let all: FormValidation<ValueMap> = {
      fields: {},
    };
    for (const val of validators) {
      const fieldVal = val(values);
      if (fieldVal) {
        all = merge(all, fieldVal);
      }
    }
    return all;
  });

  const [extraAdderActive, setExtraAdderActive] = createSignal(false);

  const availableAttrs = Object.values(props.registry.attrs).filter((attr) => {
    const ident = attr["factor/ident"];
    if (ident.startsWith("factor/")) {
      return false;
    }
    const isInEntity =
      props.schema?.["factor/entityAttributes"].find(
        (field) => field.attribute === ident
      ) !== undefined;
    return !isInEntity;
  });
  const extraAdderSearch = (term: string): Promise<AttributeSchema[]> => {
    const lower = term.toLowerCase();

    const matches = availableAttrs.filter((attr) => {
      const isMatch =
        (attr["factor/title"]?.toLowerCase().includes(lower) ?? false) ||
        attr["factor/ident"].toLowerCase().includes(lower);

      if (isMatch) {
        const isInUse =
          extraAttrs().find(
            (field) => field.schema["factor/ident"] === attr["factor/ident"]
          ) !== undefined;
        return !isInUse;
      } else {
        return false;
      }
    });

    return Promise.resolve(matches);
  };

  // FIXME: remove, just for debugging
  createEffect(() => {
    const active = extraAdderActive();
    console.log({ extraAdderActive: active });
  });

  const extraAdderToggle =
    props.forbidExtraAttributes || props.schema?.["factor/isStrict"] ? null : (
      <div class="mt-3 mb-3">
        <Show
          when={extraAdderActive()}
          fallback={
            <Button onclick={() => setExtraAdderActive(true)}>
              <Icon icon="plus" />
              Add attribute
            </Button>
          }
        >
          {() => {
            console.log("rendering extra adder picker");
            return (
              <Box>
                <p class="mb-2">
                  <b>Add attribute</b>
                </p>

                <SearchSelect<AttributeSchema>
                  itemWrapper={(props) => (
                    <div class="mb-2">
                      <Buttons>{props.children}</Buttons>
                    </div>
                  )}
                  onSelect={(schema) => {
                    const [rendered, validator] = entityAttributeFormField(
                      form,
                      schema,
                      "Optional"
                    );
                    const item: ExtraAttrItem = {
                      schema,
                      validator,
                      rendered,
                    };
                    setExtraAttrs((old) => [...old, item]);
                    setExtraAdderActive(false);
                  }}
                  onCancel={() => {
                    setExtraAdderActive(false);
                  }}
                  defaultItems={availableAttrs}
                  search={extraAdderSearch}
                  renderItem={(attr, _index, onSelect) => {
                    return (
                      <Button title={attr["factor/ident"]} onclick={onSelect}>
                        {attr["factor/ident"]}
                      </Button>
                    );
                  }}
                />

                <Buttons>
                  <Button onclick={() => setExtraAdderActive(false)}>
                    Cancel
                  </Button>
                </Buttons>
              </Box>
            );
          }}
        </Show>
      </div>
    );

  return (
    <form onsubmit={form.handleSubmit}>
      {elems}

      <For each={extraAttrs()}>{(item) => item.rendered}</For>

      {extraAdderToggle}

      <FormFooter
        form={form}
        buttons={{
          submit: { children: props.submitLabel, color: "is-info" },
          onCancel: () => {
            props.onCancel?.();
          },
        }}
      />
    </form>
  );
}
