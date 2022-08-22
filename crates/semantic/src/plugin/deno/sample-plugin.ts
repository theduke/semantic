
const SAMPLE_ID = '550e8400-e29b-11d4-a716-446655440000';
const SAMPLE_ID2 = '550e8400-e29b-11d4-a716-446655440001';
const SAMPLE_ID3 = '550e8400-e29b-11d4-a716-446655440002';

function buildData(url: string) {
    if (!(url.startsWith('http://test.com') || url.startsWith('https://test.com'))) {
      return null;
    }
    return {
      items: [
        {
          data: {
            'factor/id': SAMPLE_ID,
            'factor/type': 'semantic/Image',
            'factor/title': 'Some Image ' + url,
            'semantic/url': url,
          },
        }
      ],
      load_more_url: {url: 'https://test.com/more', label: 'Next Page'},
      related_urls: [
        { label: 'Page 1', url: 'https://test.com/page/1'},
        { label: 'Page 2', url: 'https://test.com/page/2'},
      ],
      related_items: [
        { data: {
            'factor/id': SAMPLE_ID2,
            'factor/type': 'semantic/Image',
            'factor/title': 'Related Image ',
            'semantic/url': 'https://test.com/related',
        }},
        {data: {
            'factor/id': SAMPLE_ID3,
            'factor/type': 'semantic/Image',
            'factor/title': 'Related Image ',
            'semantic/url': 'https://test.com/related',
        }},
      ],
    }
}

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
    return buildData(job.url);
  }

  import(url: string) {
    return buildData(url);
  }
}

export function buildPlugin() {
  return new Plugin();
}
