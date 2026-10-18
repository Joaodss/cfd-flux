import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    // The API and its WebSocket stream on the local cfd-server, so the app uses relative URLs.
    proxy: { '/api': { target: 'http://127.0.0.1:7878', ws: true } },
  },
})
