import { Show } from "solid-js";
import { JSX } from "solid-js/jsx-runtime";
import { Api } from "../api";
import { useApi } from "../context";
import { SemanticSchema } from "../semantic/core";
import { BoundarySuspenseLoader } from "./util/load";

export interface LoginPageProps {
  onLogin: (schema: SemanticSchema) => void;
}

async function tryLogin(
  api: Api,
  onSchemaFound: (schema: SemanticSchema) => void
): Promise<boolean> {
  const status = await api.serverStatus();
  if (status.backend_initialized) {
    const schema = await api.schema();
    onSchemaFound(schema);
    return true;
  } else {
    return false;
  }
}

export function LoginPage(props: LoginPageProps): JSX.Element {
  const api = useApi();
  return (
    <BoundarySuspenseLoader
      load={() => tryLogin(api, props.onLogin)}
      render={(flag) => <Show when={!flag}>LOGIN FORM</Show>}
    />
  );
}
