// Compatibility re-exports — the implementation now lives in ./api/.
// Existing `import { api, request, Channel, ... } from "../lib/api"` calls
// keep working without any changes.

export * from "./api/index";
