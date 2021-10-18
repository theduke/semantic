
class Plugin {
  schema() {
    return {
      name: 'sample',
    };
  }

  import(url: string) {
    return {
      items: [],
    };
  }
}

function buildPlugin(): Plugin {
  return new Plugin();
}



async function __semantic_run() {
    let res;
    try {
        const plugin = buildPlugin();
        const outputPromise = Promise.resolve(plugin.schema());
        const output = await outputPromise;
        res = {Ok: output};
    } catch (err) {
        res = {Err: err.toString()};
    }

    Deno.stderr.write(new TextEncoder().encode(JSON.stringify(res)));
}
__semantic_run();
