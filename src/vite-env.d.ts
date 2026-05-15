/// <reference types="vite/client" />

declare module "*.css" {
  const content: string;
  export default content;
}

interface Window {
  __TAURI_INTERNALS__?: unknown;
}
