import { defineConfig } from "vite-plus"
import react from "@vitejs/plugin-react"
import { lazyPlugins } from "vite-plus"

export default defineConfig({
  fmt: {
    semi: false,
  },
  lint: {
    jsPlugins: [{ name: "vite-plus", specifier: "vite-plus/oxlint-plugin" }],
    rules: { "vite-plus/prefer-vite-plus-imports": "error" },
    options: { typeAware: true, typeCheck: true },
  },
  plugins: lazyPlugins(() => [react()]),
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // Rust owns these files; watching loaded DLLs fails with EBUSY on Windows.
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: { target: "es2022", reportCompressedSize: false },
})
