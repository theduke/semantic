import { createEffect, createSignal, ErrorBoundary, JSX, Show } from "solid-js";
import {
  EntityRenderOpts,
  UiRegistry,
  ValueMap,
} from "../../semantic/registry";
import { Temporal } from "@js-temporal/polyfill";
import { Link } from "solid-app-router";
import { EntityAction, EntityBox } from "./EntityBox";
import {
  AttributeName,
  entityLinkPath,
  genericEntityTitle,
} from "../../semantic";
import {
  BaseEntity,
  FACTOR_ID,
  FACTOR_IDENT,
  FACTOR_TITLE,
  FACTOR_TYPE,
  FACTOR_VALUE_TYPE,
} from "semantic/dist/schema";
import { EntitySchema } from "semantic/dist/core";
import { EntityChildren } from "./EntityChildren";
import { EntityEditor } from "./EntityEditor";
import { EntityDeleterModal } from "./EntityDeleterModal";
import { NotificationError, NotificationWarning } from "../bulma/notification";
import { Button } from "../bulma/button";

export function rendererValue(value: any): JSX.Element {
  const ty = typeof value;
  switch (ty) {
    case "string":
      return value;
    case "number":
      return value;
    case "bigint":
      return value;
    case "boolean":
      return value ? "yes" : "no";
    case "symbol":
      return value;
    case "undefined":
      return null;
    case "function":
      return "<function>";
    case "object":
      if (value === null) {
        return null;
      } else if (Array.isArray(value)) {
        const values = value.map((v) => <li>{rendererValue(v)}</li>);
        return <ul>{values}</ul>;
      } else {
        // FIXME: render generic objects!
        return JSON.stringify(value, null, 2);
      }
  }
}

export function renderAttrValue(
  reg: UiRegistry,
  attr: AttributeName,
  value: any
): JSX.Element {
  const attrty = reg.attrs[attr];
  if (!attrty) {
    return rendererValue(value);
  }
  const valty = attrty[FACTOR_VALUE_TYPE];
  const ty = typeof value;
  switch (valty) {
    case "Any":
      return value;
    case "Unit":
      return null;
    case "Bool":
      return value ? "yes" : "no";
    case "Int":
      return value;
    case "UInt":
      return value;
    case "Float":
      return value;
    case "String":
      return value;
    case "Bytes":
      if (Array.isArray(value)) {
        return `bytearray[len=${value.length}]`;
      } else {
        return value.toString();
      }
    case "DateTime":
      return new Temporal.Instant(BigInt(value)).toLocaleString();
    case "Url":
      return (
        <a href={value} target="_blank">
          {value}
        </a>
      );
    case "Ref":
      return <Link href={"/entity/" + value}>{value}</Link>;
    default:
      if (ty === "object") {
        if ("Const" in value) {
          return rendererValue(value["Const"]);
        } else if ("Map" in value) {
          // FIXME: render map!
          return JSON.stringify(value, null, 2);
        } else if ("Union" in value) {
          // FIXME: render union!
          return JSON.stringify(value, null, 2);
        } else if ("Object" in value) {
          // FIXME: render object!
          return JSON.stringify(value, null, 2);
        }
      }
  }

  return rendererValue(value);
}

export function renderEntityTable(
  reg: UiRegistry,
  item: ValueMap
): JSX.Element {
  const rows = Object.entries(item).map(([attr, value]) => {
    const name = reg.attrs[attr]?.["factor/title"] ?? attr;
    const rendered = renderAttrValue(reg, attr, value);
    return (
      <tr>
        <td>
          <span title={attr}>{name}</span>
        </td>
        <td>{rendered}</td>
      </tr>
    );
  });

  const children = (
    <tr>
      <td>Children</td>
      <td>
        <EntityChildren id={item[FACTOR_ID]} />
      </td>
    </tr>
  );
  rows.push(children);

  return <table class="table">{rows}</table>;
}

export function renderEntityBox(
  reg: UiRegistry,
  schema: EntitySchema,
  item: ValueMap,
  opts: EntityRenderOpts
): JSX.Element {
  // TODO: wrap in nested function to cache lookups.
  const ident = schema["factor/ident"];
  const typeName = schema["factor/title"] ?? ident;
  const contentRender = reg.entityContentRenderers[ident];

  const content = contentRender
    ? contentRender(item, opts)
    : renderEntityTable(reg, item);

  return (
    <EntityBox
      linkPath={entityLinkPath(item)}
      title={genericEntityTitle(item)}
      type={typeName}
    >
      {content}
    </EntityBox>
  );
}

export function renderGenericEntityBox(
  reg: UiRegistry,
  item: ValueMap,
  opts: EntityRenderOpts
): JSX.Element {
  const ty = item[FACTOR_TYPE];
  const schema = reg.entityTypes[ty];

  const typeName = schema
    ? schema?.[FACTOR_TITLE] ?? schema?.[FACTOR_IDENT] ?? ty
    : ty;

  const ident = schema?.["factor/ident"];

  const [getItem, setItem] = createSignal<ValueMap>(item);
  const [deleted, setDeleted] = createSignal<boolean>(false);

  let actions: EntityAction[] = [];

  if (opts.allowEdit) {
    const editAction: EntityAction = {
      label: "Edit",
      icon: "pencil",
      replacesContent: true,
      render: (props) => {
        return (
          <div>
            <ErrorBoundary
              fallback={(err: any) => (
                <NotificationError>
                  <div class="mb-4">
                    Could not render form:
                    <br />
                    {err.toString()}
                  </div>
                  <div>
                    <Button
                      onClick={() => {
                        props.close(editAction);
                      }}
                    >
                      Okay
                    </Button>
                  </div>
                </NotificationError>
              )}
            >
              <EntityEditor
                item={getItem()}
                onSaved={(item) => {
                  setItem(item);
                  props.close(editAction);
                  opts.onModified?.(item);
                }}
              />
            </ErrorBoundary>
          </div>
        );
      },
    };
    actions.push(editAction);
  }
  if (opts.allowDelete) {
    const deleteAction: EntityAction = {
      label: "Delete",
      icon: "trash",
      replacesContent: false,
      render: (props) => {
        return (
          <EntityDeleterModal
            entity={getItem() as BaseEntity}
            onDeleted={() => {
              setDeleted(true);
              props.close(deleteAction);
              opts.onDeleted?.(getItem());
            }}
            onCancel={() => {
              props.close(deleteAction);
            }}
          />
        );
      },
    };
    actions.push(deleteAction);
  }

  let [content, setContent] = createSignal<JSX.Element>(null);

  let render: (item: ValueMap) => JSX.Element;
  const contentRender = reg.entityContentRenderers[ident];

  if (!!contentRender) {
    render = (item) => contentRender(item, opts);

    actions.push({
      label: "Show table",
      icon: "table",
      replacesContent: true,
      render: () => {
        return renderEntityTable(reg, getItem());
      },
    });
  } else {
    render = (item) => renderEntityTable(reg, item);
  }

  setContent(render(getItem()));
  createEffect(() => {
    console.debug('replacting entity box content');
    setContent(render(getItem()));
  });

  const box = (
    <EntityBox
      linkPath={entityLinkPath(item)}
      title={genericEntityTitle(getItem())}
      type={typeName}
      actions={actions}
    >
      {content()}
    </EntityBox>
  );

  return (
    <Show when={deleted()} fallback={box}>
      <NotificationWarning>Item was deleted.</NotificationWarning>
    </Show>
  );
}
