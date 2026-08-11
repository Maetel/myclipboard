import { defineConfig } from 'vite';

export default defineConfig({
  clearScreen: false,
  server: {
    port: 1421,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/target/**'] },
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    target: ['es2021', 'chrome105', 'safari13'],
    rollupOptions: { input: ['index.html', 'popup.html'] },
  },
});
