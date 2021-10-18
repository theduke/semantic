
class Plugin {
  static NAME = 'sample';
  schema() {
    return {
      name: Plugin.NAME,
    };
  }

  import(url: string) {

    if (url === 'http://test.com/abc') {
      return {
        plugin: Plugin.NAME,
        items: [
          {
            data: {
              'factor/type': 'semantic/Image',
              'factor/title': 'Some Image',
            },
          }
        ],
      };
    } else {
      return null;
    }
  }
}

export function buildPlugin() {
  return new Plugin();
}
