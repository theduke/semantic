import { readLines } from "https://deno.land/std@0.76.0/io/bufio.ts";

let PLUGIN: any|null = null;

type PluginCommand =
  | { Init: { plugin_path: string } }
  | "Ping"
  | { Import: { url: String } };

type PluginReply =
  | { Init: { schema: any } }
  | "Ping"
  | { Import: { output: any | null } };

async function run() {
  const reader = readLines(Deno.stdin);

  for await (const line of reader) {
    await tryHandleLine(line);
  }
}

async function tryHandleLine(line: string) {
  let response: any;
  try {
    const reply = await handleLine(line);
    response = {Ok: reply};
  } catch (err) {
    response = {Err: err.toString()};
  }

  const output = JSON.stringify(response) + '\n';

  await Deno.stderr.write(new TextEncoder().encode(output));
}

async function handleLine(line: string) {
  const command: PluginCommand = JSON.parse(line);

  let reply: PluginReply;
  if (command === "Ping") {
    reply = "Ping";
  } else if ("Init" in command) {
    const module = await import(command.Init.plugin_path);
    PLUGIN = module.buildPlugin();
    const schema = PLUGIN.schema();
    reply = {Init: {schema}};
  } else if ("Import" in command) {
    const output = await PLUGIN.import(command.Import.url);
    reply = {Import: {output}};
  } else {
    throw new Error('Invalid command' + JSON.stringify(command));
  }

  return reply;
}

run();
