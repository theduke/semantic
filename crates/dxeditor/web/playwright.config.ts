import { defineConfig, devices } from '@playwright/test'

const chromiumPath = process.env.PLAYWRIGHT_CHROMIUM_PATH || undefined
const firefoxPath = process.env.PLAYWRIGHT_FIREFOX_PATH || undefined
const webkitPath = process.env.PLAYWRIGHT_WEBKIT_PATH || undefined

export default defineConfig({
  testDir: './e2e',
  testMatch: /.*\.spec\.ts/,
  fullyParallel: false,
  retries: 0,
  reporter: 'line',
  use: {
    baseURL: 'http://127.0.0.1:4173',
    trace: 'retain-on-failure',
  },
  webServer: {
    command: 'python3 -m http.server 4173 --bind 127.0.0.1',
    port: 4173,
    reuseExistingServer: true,
  },
  projects: [
    {
      name: 'chromium',
      use: {
        ...devices['Desktop Chrome'],
        launchOptions: chromiumPath ? { executablePath: chromiumPath } : {},
      },
    },
    {
      name: 'firefox',
      use: {
        ...devices['Desktop Firefox'],
        launchOptions: firefoxPath ? { executablePath: firefoxPath } : {},
      },
    },
    {
      name: 'webkit',
      use: {
        ...devices['Desktop Safari'],
        launchOptions: webkitPath ? { executablePath: webkitPath } : {},
      },
    },
  ],
})
