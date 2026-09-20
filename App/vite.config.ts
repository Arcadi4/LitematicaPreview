import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // Rust owns these files; watching loaded DLLs fails with EBUSY on Windows.
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: { target: "es2022", reportCompressedSize: false },
});
