import { defineConfig } from "cf/config"

export default defineConfig({
  accountId: "9a69f1d9357de40a684b037f0a7405ae",
  worker: {
    name: "apps-update",
    compatibilityDate: "2026-11-01",
    entrypoint: "./index.ts",
    domains: ["update.tanh.moe"],
    workersDev: false,
  },
})
