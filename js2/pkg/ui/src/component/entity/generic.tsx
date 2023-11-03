import {
    FACTOR_ID,
  FACTOR_TYPE,
  FACTOR_VALUE_TYPE,
  ValueMap,
  genericEntityTitle,
} from '@semantic/api';
import { ReactNode } from 'react';
import { Temporal } from '@js-temporal/polyfill';
import { EntityRenderOpts, UiRegistry } from '../../registry';
import { EntityBox } from './EntityBox';
import { Class, ValueType } from '@semantic/api/src/core';
import { AttrName } from '@semantic/api/src/db';

export function renderValue(value: any): ReactNode {
  const ty = typeof value;
  switch (ty) {
    case 'string':
      return value;
    case 'number':
      return value;
    case 'bigint':
      return value;
    case 'boolean':
      return value ? 'yes' : 'no';
    case 'symbol':
      return value;
    case 'undefined':
      return null;
    case 'function':
      return '<function>';
    case 'object':
      if (value === null) {
        return null;
      } else if (Array.isArray(value)) {
        const values = value.map((v) => <li>{renderValue(v)}</li>);
        return (
          <div className="content">
            <ul>{values}</ul>
          </div>
        );
      } else {
        // FIXME: render generic objects!
        return JSON.stringify(value, null, 2);
      }
  }
}

export function renderTypedValue(value: any, ty: ValueType): ReactNode {
  if (value === null || value === undefined) {
    return null;
  }

  const valty = typeof value;

  switch (ty) {
    case 'Any':
    case 'Int':
    case 'UInt':
    case 'Float':
    case 'String':
    case 'Unit':
      break;

    case 'Bool':
      if (valty === 'boolean') {
        return value ? 'yes' : 'no';
      }
      break;

    case 'Bytes':
      if (Array.isArray(value)) {
        return `bytearray[len=${value.length}]`;
      }
      break;

    case 'DateTime':
      if (valty === 'number') {
        return new Temporal.Instant(BigInt(value * 1_000_000)).toLocaleString();
      }
      break;

    case 'Url':
      if (valty === 'string') {
        return (
          <a href={value} target="_blank">
            {value}
          </a>
        );
      }
      break;

    case 'Ref':
      if (typeof value === 'string') {
        // TODO: use link component!
        return <a href={'/entity/' + value}>{value}</a>;
      }
      break;

    default:
      break;
  }

  if (typeof ty === 'object') {
    if ('List' in ty && Array.isArray(value)) {
      const itemTy = ty['List'];
      console.log({ itemTy, value });
      const items = value.map((v) => <li>{renderTypedValue(v, itemTy)}</li>);
      return (
        <div className="content">
          <ul style={{ marginTop: 0 }}>{items}</ul>
        </div>
      );
    }
  } else if (valty === 'object') {
    if ('Map' in value) {
      // FIXME: render map!
      return JSON.stringify(value, null, 2);
    } else if ('Union' in value) {
      // FIXME: render union!
      return JSON.stringify(value, null, 2);
    } else if ('Object' in value) {
      // FIXME: render object!
      return JSON.stringify(value, null, 2);
    }
  }

  return renderValue(value);
}

export function renderAttrValue(
  reg: UiRegistry,
  attr: AttrName,
  value: any,
  entity: ValueMap,
): ReactNode {
  const attrty = reg.attrs[attr];
  if (attrty) {
    const customRenderer = reg.attributeRenderers[attr];
    if (customRenderer) {
      return customRenderer(value, entity);
    }
    return renderTypedValue(value, attrty[FACTOR_VALUE_TYPE]);
  } else {
    return renderValue(value);
  }
}

export function renderEntityTable(reg: UiRegistry, item: ValueMap): ReactNode {
  const rows = Object.entries(item).map(([attr, value]) => {
    const name = reg.attrs[attr]?.['factor/title'] ?? attr;
    const rendered = renderAttrValue(reg, attr, value, item);
    return (
      <tr>
        <td>
          <span title={attr}>{name}</span>
        </td>
        <td>{rendered}</td>
      </tr>
    );
  });

  // const id = item[FACTOR_ID];
  // const [showChildren, setShowChildren] = createSignal(false);

  // const children = id ? (
  //   <tr>
  //     <td>Children</td>
  //     <td>
  //       <Show
  //         when={showChildren()}
  //         fallback={
  //           <Button onclick={() => setShowChildren(true)}>Load children</Button>
  //         }
  //       >
  //         <EntityChildrenLoader
  //           id={item[FACTOR_ID]}
  //           render={(item) => {
  //             return (
  //               <Link href={entityLinkPath(item)}>{reg.entityTitle(item)}</Link>
  //             );
  //           }}
  //         />
  //       </Show>
  //     </td>
  //   </tr>
  // ) : null;
  // rows.push(children);

  return (
    <table className="table" style={{ overflowY: 'scroll' }}>
      {rows}
    </table>
  );
}

export function renderEntityBox(
  reg: UiRegistry,
  schema: Class,
  item: ValueMap,
  opts: EntityRenderOpts,
): ReactNode {
  // TODO: wrap in nested function to cache lookups.
  const ident = schema['factor/ident'];
  const typeName = <span title={ident}>{schema['factor/title'] ?? ident}</span>;
  const contentRender = reg.entityContentRenderers[ident];

  const content = contentRender
    ? contentRender(item, opts)
    : renderEntityTable(reg, item);

  return (
    <EntityBox
      // linkPath={entityLinkPath(item)}
      id={item[FACTOR_ID]}
      title={genericEntityTitle(item)}
      entityType={typeName}
    >
      {content}
    </EntityBox>
  );
}

export function renderGenericEntityBox(
  reg: UiRegistry,
  item: ValueMap,
  opts: EntityRenderOpts,
): ReactNode {
  const ty = item[FACTOR_TYPE];
  const schema = reg.classes[ty];

  // const typeName = schema
  //   ? schema?.[FACTOR_TITLE] ?? schema?.[FACTOR_IDENT] ?? ty
  //   : ty;

  // const ident = schema?.['factor/ident'];

  return renderEntityBox(reg, schema, item, opts);

  // const [getItem, setItem] = createSignal<ValueMap>(item);
  // const [deleted, setDeleted] = createSignal<boolean>(false);

  // const [content, setContent] = createSignal<ReactNode>(null);
  // const actions: EntityAction[] = [];

  // let render: (item: ValueMap) => ReactNode;
  // const contentRender = reg.entityContentRenderers[ident];

  // if (contentRender) {
  //   render = (item) => contentRender(item, opts);

  //   actions.push({
  //     label: 'Show table',
  //     icon: 'table',
  //     replacesContent: true,
  //     render: () => {
  //       return renderEntityTable(reg, getItem());
  //     },
  //   });
  // } else {
  //   render = (item) => renderEntityTable(reg, item);
  // }

  // const externalUrl: string = item[SEMANTIC_URL];
  // if (externalUrl && externalUrl.startsWith('http')) {
  //   const externalUrlAction: EntityAction = {
  //     label: 'Open external url',
  //     icon: 'globe',
  //     replacesContent: false,
  //     render: () => null,
  //     renderAction: () => {
  //       return (
  //         <a class="button is-small" target="_blank" href={externalUrl}>
  //           <Icon icon="globe" />
  //         </a>
  //       );
  //     },
  //   };
  //   actions.push(externalUrlAction);
  // }

  // const tagManagerAction: EntityAction = {
  //   label: 'Tags',
  //   icon: 'tags',
  //   replacesContent: false,
  //   render: (props) => {
  //     return (
  //       <EntityTagManager
  //         entity={getItem()}
  //         modal
  //         onFinished={(newTags) => {
  //           if (newTags !== null) {
  //             setItem((old) => ({ ...old, [SEMANTIC_TAGS]: newTags }));
  //           }
  //           props.close(tagManagerAction);
  //         }}
  //       />
  //     );
  //   },
  // };
  // actions.push(tagManagerAction);

  // if (opts.allowEdit) {
  //   const editAction: EntityAction = {
  //     label: 'Edit',
  //     icon: 'pencil',
  //     replacesContent: true,
  //     render: (props) => {
  //       return (
  //         <div>
  //           <ErrorBoundary
  //             fallback={(err: any) => (
  //               <NotificationError>
  //                 <div class="mb-4">
  //                   Could not render form:
  //                   <br />
  //                   {err.toString()}
  //                 </div>
  //                 <div>
  //                   <Button
  //                     onClick={() => {
  //                       props.close(editAction);
  //                     }}
  //                   >
  //                     Okay
  //                   </Button>
  //                 </div>
  //               </NotificationError>
  //             )}
  //           >
  //             <EntityEditor
  //               item={getItem()}
  //               onSaved={(item) => {
  //                 setItem(item);
  //                 props.close(editAction);
  //                 opts.onModified?.(item);
  //               }}
  //             />
  //           </ErrorBoundary>
  //         </div>
  //       );
  //     },
  //   };
  //   actions.push(editAction);
  // }
  // if (opts.allowDelete) {
  //   const deleteAction: EntityAction = {
  //     label: 'Delete',
  //     icon: 'trash',
  //     replacesContent: false,
  //     render: (props) => {
  //       return (
  //         <EntityDeleterModal
  //           entity={getItem() as BaseEntity}
  //           onDeleted={() => {
  //             setDeleted(true);
  //             props.close(deleteAction);
  //             opts.onDeleted?.(getItem());
  //           }}
  //           onCancel={() => {
  //             props.close(deleteAction);
  //           }}
  //         />
  //       );
  //     },
  //   };
  //   actions.push(deleteAction);
  // }

  // setContent(render(getItem()));
  // createEffect(() => {
  //   setContent(render(getItem()));
  // });

  // const title = genericEntityTitle(getItem());

  // return (
  //   <EntityBox
  //     // linkPath={entityLinkPath(item)}
  //     title={title}
  //     entityType={typeName}
  //     // actions={actions}
  //   >
  //     {content()}
  //   </EntityBox>
  // );
}
