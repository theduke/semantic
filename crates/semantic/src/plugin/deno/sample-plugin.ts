
const SAMPLE_ID = '550e8400-e29b-11d4-a716-446655440000';

class Plugin {
  static NAME = 'sample';
  schema() {
    return {
      name: Plugin.NAME,
      import_matchers: [
        {
          matcher: {Domains: {domains: ["test.com"]}},
          support: 'Dedicated',
        }
      ],
    };
  }

  fetchUrl(job: any) {
    const url = job.url;
    if (!(url.startsWith('http://test.com') || url.startsWith('https://test.com'))) {
      return null;
    }
    return {
      items: [
        {
          data: {
            'factor/type': 'semantic/Image',
            'factor/title': 'Some Image ' + url,
            'semantic/url': url,
          },
        }
      ],
      load_more_url: null,
      related_urls: [],
      related_items: [],
    }
  }

  import(url: string) {
    if (!(url.startsWith('http://test.com') || url.startsWith('https://test.com'))) {
      return null;
    }

    return {
      items: [
        {
          data: {
            'factor/type': 'semantic/Image',
            'factor/title': 'Some Image ' + url,
            'semantic/url': url,
          },
        }
      ],
    };
  }
}

export function buildPlugin() {
  return new Plugin();
}
