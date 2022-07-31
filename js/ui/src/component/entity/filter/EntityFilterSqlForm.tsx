import { Accessor, JSX, Setter } from "solid-js";
import { createForm, FormValidation } from "../../form";
import { TextAreaField } from "../../form/TextAreaField";

import zod from "zod";

export const validateEntityFilterSql = zod.object({
  type: zod.literal("sql"),
  sql: zod.string(),
});

export type EntityFilterSql = zod.infer<typeof validateEntityFilterSql>;

export interface EntityFilterSqlFormProps {
  filter: Accessor<EntityFilterSql>;
  setFilter: Setter<EntityFilterSql>;
}

interface Values {
  sql: string;
}

export function EntityFilterSqlForm(
  props: EntityFilterSqlFormProps
): JSX.Element {
  const form = createForm<Values>({
    initialValues: { sql: props.filter().sql },
    validate: (values) => {
      if (!values.sql.trim()) {
        const val: FormValidation<Values> = {
          fields: { sql: { errors: [{ message: "Statement is required" }] } },
        };
        return val;
      } else {
        return null;
      }
    },
    onValid: (values) => {
      props.setFilter({ type: "sql", sql: values.sql });
    },
  });

  return (
    <form onsubmit={(e) => {}}>
      <TextAreaField
        field={form.field("sql")}
        label={null}
        help="Enter a SQL query"
      />
    </form>
  );
}
