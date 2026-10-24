import { defineConfig, devices } from "@playwright/test";

// HUSH_E2E_PORT when 4173 is taken by something else on the machine.
const PORT = Number(process.env.HUSH_E2E_PORT ?? 4173);

export default defineConfig({
  testDir: "e2e",
  timeout: 120_000,
  expect: { timeout: 20_000 },
  fullyParallel: false,
  workers: 2,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["github"], ["list"]] : "list",
  use: {
    baseURL: `http://localhost:${PORT}`,
    trace: "retain-on-failure",
  },
  webServer: {
    command: "bun test/e2e-server.ts",
    url: `http://localhost:${PORT}/`,
    reuseExistingServer: !process.env.CI,
    env: { PORT: String(PORT) },
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    // Playwright's Linux WebKit cannot open WebRTC connections: CI runs this project on macOS.
    {
      name: "webkit",
      use: { ...devices["Desktop Safari"] },
      testMatch: /(direct|manual)\.spec\.ts/,
    },
    { name: "mobile", use: { ...devices["Pixel 7"] }, testMatch: /layout\.spec\.ts/ },
  ],
});
