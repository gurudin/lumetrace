import { fileURLToPath } from "node:url";
import { createViteConfig } from "./build/createViteConfig";

const root = fileURLToPath(new URL(".", import.meta.url));
export default createViteConfig({ root, sharedRoot: root });
