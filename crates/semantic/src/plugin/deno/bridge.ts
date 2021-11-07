import { readLines } from "https://deno.land/std@0.76.0/io/bufio.ts";

let PLUGIN: any | null = null;

type PluginCommand =
  | { Init: { plugin_path: string } }
  | "Ping"
  | { FetchUrl: any }
  | { Import: any };

type PluginReply =
  | { Init: { schema: any } }
  | "Ping"
  | { FetchUrl: { output: any | null } }
  | { Import: { output: any } };

async function run() {
  const reader = readLines(Deno.stdin);

  for await (const line of reader) {
    await tryHandleLine(line);
  }
}

async function tryHandleLine(line: string) {
  if (line === "") {
    return;
  }
  let response: any;
  try {
    const reply = await handleLine(line);
    response = { Ok: reply };
  } catch (err) {
    response = { Err: err.toString() };
  }

  console.log({ sendingResponse: response });
  let output = JSON.stringify(response) + "\n";

  Deno.stderr.writeSync(new TextEncoder().encode(output));
  console.log("reply written");
}

async function handleLine(line: string) {
  const command: PluginCommand = JSON.parse(line);

  let reply: PluginReply;
  console.log({ runningCommand: command });
  if (command === "Ping") {
    reply = "Ping";
  } else if ("Init" in command) {
    const module = await import(command.Init.plugin_path);
    PLUGIN = module.buildPlugin();
    const schema = PLUGIN.schema();
    reply = { Init: { schema } };
  } else if ("FetchUrl" in command) {
    const output = await PLUGIN.fetchUrl(command.FetchUrl);
    reply = { FetchUrl: { output } };
  } else if ("Import" in command) {
    const output = await PLUGIN.fetchUrl(command.Import);
    reply = { Import: { output } };
  } else {
    throw new Error("Invalid command" + JSON.stringify(command));
  }

  return reply;
}

run();
