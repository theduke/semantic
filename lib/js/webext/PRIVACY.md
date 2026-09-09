# Privacy and permissions

The Semantic browser extension does not include analytics, advertising, or
third-party network services. Its data stays in browser extension storage and the
Semantic server you configure.

The extension reads the active tab's URL and title only when you open its popup.
It sends the exact URL to the configured Semantic server to find an existing
bookmark. If you save, it also sends the title and any description you entered.
Firefox classifies those values as `browsingActivity` and `websiteContent`, which
the Firefox manifest declares as required data collection categories.

Required permissions are limited to:

- `activeTab`, to inspect the tab for which you opened the popup.
- `storage`, to retain the configured Semantic application URL locally.

Network access is optional. Saving settings asks the browser to grant access only
to the configured server origin. Changing origins removes the previous grant after
the new configuration is stored. No page content scripts are injected.
