import { Show } from "solid-js";
import { JSX } from "solid-js/jsx-runtime";
import { Api } from "semantic/dist/api";
import { useApi } from "../context";
import { SemanticSchema } from "semantic/dist/core";
import { BoundarySuspenseLoader, FallibleResourceLoader } from "./util/load";

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
    <FallibleResourceLoader
      load={() => tryLogin(api, props.onLogin)}
      children={(flag) => <Show when={!flag}>LOGIN FORM</Show>}
    />
  );
}
