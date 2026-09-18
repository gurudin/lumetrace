import { fileURLToPath } from "node:url";
import { createViteConfig } from "../../build/createViteConfig";

const root = fileURLToPath(new URL("../..", import.meta.url));
const config = createViteConfig({ root, sharedRoot: root });
// The application startup shell intentionally replaces every HTML entry.
// Only the isolated fixture server bypasses that shell; production is unchanged.
export default {
  ...config,
  plugins: config.plugins?.filter(plugin => (plugin as { name?: string })?.name !== "lumetrace-shared-startup-shell"),
};
