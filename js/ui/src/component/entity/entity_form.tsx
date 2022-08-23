import { merge } from "lodash";
import { Box } from "solid-bulma";
import { createEffect, createSignal, For, JSX, Show } from "solid-js";
import {
  Attribute,
  Cardinality,
  Class,
  Expr,
  Id,
  Select,
  Value,
  ValueType,
} from "semantic/dist/core";
import { exprAttr, exprIn, exprLiteral, exprNotEq } from "semantic/dist/db";
import { UiRegistry, ValueMap } from "../../semantic/registry";
import {
  FACTOR_ENTITY_ATTRIBUTES,
  FACTOR_ID,
  FACTOR_IDENT,
  FACTOR_TITLE,
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
import { MultiEntityPicker } from "./MultiEntityPicker";
import { newSelect } from "semantic/dist/api";
import { EntitiesLoader } from "./EntitiesLoader";
import { FormField } from "../form/FormField";

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
  required: boolean,
  validate?: (value: string) => string | null
): FormValidator<ValueMap> {
  if (required) {
    return (values) => {
      const value = values[attr]?.toString().trim();

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
        if (validate) {
          const msg = validate(value);
          if (msg) {
            return {
              fields: {
                [attr]: {
                  errors: [
                    {
                      message: msg,
                    },
                  ],
                },
              },
            };
          } else {
            return null;
          }
        } else {
          return null;
        }
      }
    };
  } else {
    return (_values) => null;
  }
}

export function entityAttributeFormField(
  form: FormState<ValueMap>,
  attr: Attribute,
  cardinality: Cardinality
): [JSX.Element, FormValidator<ValueMap>] {
  const attributeName = attr["factor/title"] || attr["factor/ident"];
  const ty = attr["factor/valueType"];

  const attrIdent = attr["factor/ident"];

  const isRequired = cardinality === "Required";

  if (ty === "Bool") {
    let field = form.field(attrIdent) as FieldAccessor<boolean>;
    return [
      // TODO: better checkboxLabel?
      <CheckboxField
        label={attributeName}
        checkboxLabel={"True/False"}
        field={field}
      />,
      (_values) => null,
    ];
  } else if (
    ty === "String" ||
    ty === "Url" ||
    ty == "UInt" ||
    ty === "Int" ||
    ty === "Float"
  ) {
    const field = form.field(attrIdent) as FieldAccessor<string>;

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

      // TODO: int and uint should not use strings as values!
      case "Int":
        inputType = "number";
        // TODO: validate correct URL
        val = makeStringValidator(attrIdent, isRequired, (value) => {
          try {
            parseInt(value);
            return null;
          } catch (err: any) {
            return err.toString();
          }
        });
        break;

      case "UInt":
        inputType = "number";
        // TODO: validate correct URL
        val = makeStringValidator(attrIdent, isRequired, (value) => {
          try {
            const num = parseInt(value);
            if (num < 0) {
              return "must be 0 or greater";
            }
            return null;
          } catch (err: any) {
            return err.toString();
          }
        });
        break;

      case "Float":
        inputType = "number";
        // TODO: validate correct URL
        val = makeStringValidator(attrIdent, isRequired, (value) => {
          try {
            parseFloat(value);
            return null;
          } catch (err: any) {
            return err.toString();
          }
        });
        break;

      default:
        throw new Error(`Unsupported value type: ${ty}`);
    }

    const elem = (
      <InputField type={inputType} label={attributeName} field={field} />
    );

    return [elem, val];
  } else if (ty === "DateTime") {
    const field = form.field(attrIdent) as FieldAccessor<number>;
    const validator = (values: ValueMap) => {
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

    const elem = <DateTimeInputField label={attributeName} field={field} />;

    return [elem, validator];
  } else if (ty === "Ref") {
    const field = form.field(attrIdent) as FieldAccessor<Id | undefined>;

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
  } else if (typeof ty === "object" && "List" in ty) {
    const itemType = ty["List"];

    if (itemType === "Ref") {
      const field = form.field(attrIdent) as FieldAccessor<Id[] | undefined>;
      const id = form.init.initialValues[FACTOR_ID];

      const baseFilter: Expr | undefined = id
        ? exprNotEq(exprAttr(FACTOR_ID), exprLiteral(id))
        : undefined;

      // TODO: make reactive!
      const initialValues = field.get()?.value || [];
      const currentItemsFilter = exprIn(
        exprAttr(FACTOR_ID),
        exprLiteral(initialValues)
      );
      const currentItemsSelect: Select = {
        ...newSelect(),
        filter: currentItemsFilter,
        limit: 1000 as any,
      };
      const elem = (
        <EntitiesLoader select={currentItemsSelect}>
          {(initialItems) => {
            const picker = (
              <MultiEntityPicker
                baseFilter={baseFilter}
                initialSelection={initialItems}
                onChange={(items) => {
                  const ids = items.map((item) => item[FACTOR_ID]);
                  field.set(ids);
                }}
              />
            );
            const elem = (
              <FormField field={field} label={attributeName} control={picker} />
            );

            return elem;
          }}
        </EntitiesLoader>
      );
      return [elem, (_) => null];
    } else {
      throw new Error(
        `Could not create form field: unsupported attribute type for attribute ${attrIdent}: ${JSON.stringify(
          ty
        )}`
      );
    }
  } else if (typeof ty === "object") {
    // FIXME: implement support!
    if ("Union" in ty) {
      const options: SelectOption<Value | undefined>[] = [
        { label: "", value: undefined },
        ...unionPlainOptions(ty["Union"]),
      ];
      const field = form.field(attrIdent) as FieldAccessor<Value>;

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
    `Could not create form field: unsupported attribute type for attribute ${attrIdent}: ${JSON.stringify(
      ty
    )}`
  );
}

export interface GenericEntityFormProps {
  registry: UiRegistry;
  schema?: Class | null;

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
  schema: Attribute;
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
    const ident = attr[FACTOR_IDENT];
    if (ident.startsWith("factor/")) {
      return false;
    }
    const isInEntity =
      props.schema?.[FACTOR_ENTITY_ATTRIBUTES].find(
        (field) => field["factor/attribute"] === ident
      ) !== undefined;
    return !isInEntity;
  });
  const extraAdderSearch = (term: string): Promise<Attribute[]> => {
    const lower = term.toLowerCase();

    const matches = availableAttrs.filter((attr) => {
      const isMatch =
        (attr[FACTOR_TITLE]?.toLowerCase().includes(lower) ?? false) ||
        attr[FACTOR_IDENT].toLowerCase().includes(lower);

      if (isMatch) {
        const isInUse =
          extraAttrs().find(
            (field) => field.schema[FACTOR_IDENT] === attr[FACTOR_IDENT]
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

                <SearchSelect<Attribute>
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
