/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_QUIVERDL_UPDATER?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
