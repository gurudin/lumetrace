import { readFileSync } from "node:fs";
import { relative, resolve } from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { renderApplicationHtml } from "./applicationHtml.ts";

/** Shared build behavior; the caller owns its entry, output and edition identity. */
export function createViteConfig(options: {
  root: string;
  sharedRoot: string;
  displayName?: string;
  port?: number;
}) {
  const { root, sharedRoot, displayName = "LumeTrace", port = 1420 } = options;
  const host = process.env.TAURI_DEV_HOST;
  const sharedPath = relative(root, sharedRoot).replaceAll("\\", "/");
  if (sharedPath.startsWith("..")) {
    throw new Error("The shared source must be the app root or a pinned dependency inside it.");
  }
  return defineConfig({
    root,
    plugins: [
      {
        name: "lumetrace-shared-startup-shell",
        transformIndexHtml: {
          order: "pre",
          handler: () => renderApplicationHtml(readFileSync(resolve(sharedRoot, "index.html"), "utf8"), {
            displayName,
            assetPrefix: sharedPath ? `/${sharedPath}` : "",
          }),
        },
      },
      react(),
      tailwindcss(),
    ],
    resolve: {
      alias: {
        react: resolve(sharedRoot, "node_modules/react"),
        "react-dom": resolve(sharedRoot, "node_modules/react-dom"),
      },
    },
    clearScreen: false,
    build: { outDir: resolve(root, "dist"), emptyOutDir: true },
    server: {
      port,
      strictPort: true,
      host: host || false,
      hmr: host ? { protocol: "ws", host, port: port + 1 } : undefined,
      fs: { allow: [root] },
      watch: { ignored: ["**/src-tauri/**"] },
    },
  });
}
