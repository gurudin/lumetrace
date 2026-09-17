// Synthetic browser harness. No application startup, live DB or credentials.
import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import { FileSpaceSearchPanel } from "../../src/pages/file-space/FileSpaceSearchPanel";
import { ThemeProvider } from "../../src/shared/theme/ThemeProvider";
import "../../src/shared/i18n/i18n";
import "../../src/styles.css";

const fixtures = await fetch("/__fixtures/payload").then(r => r.json());
const records = fixtures.map((entry: any) => entry.file);
const files = [...records, ...Array.from({ length: 60 }, (_, i) => ({...records[0],id:`density-${i}`,name:`Long realistic research screenshot ${i+1} — quarterly project review and OCR references.png`}))];
const calls: Record<string,number> = {};
Object.assign(window, {
  isTauri: true,
  __visualCalls: calls,
  __TAURI_INTERNALS__: {
    convertFileSrc: (id: string) => `/__fixtures/${files.find(file => file.id === id)?.name ?? "missing"}`,
    invoke: async (command: string, args: any) => {
      calls[command] = (calls[command] ?? 0) + 1;
      if (command === "search_file_space_files") return files.map(file => ({ fileId:file.id, lexicalMatch:true, semanticSimilarity:null, contentMatch:args.request.query !== "filename", snippet:args.request.query === "filename" ? null : "ORCHID September 20 项目预算 北京 工作计划" }));
      if (command === "list_file_space_files") return { files };
      if (command === "get_file_space_search_preview") {
        const entry = fixtures.find((entry: any) => entry.file.id === args.request.fileId) ?? fixtures[0];
        if ((window as any).__delayPreview) await new Promise(resolve => setTimeout(resolve, 500));
        return {...entry.preview, fileId:args.request.fileId};
      }
      if (command === "read_file_space_pdf") {
        const file = files.find(file => file.id === args.fileId);
        const response = await fetch(`/__fixtures/${file.name}`);
        if (!response.ok) throw new Error("Synthetic transport failure");
        return response.arrayBuffer();
      }
      throw new Error(`Unexpected synthetic command: ${command}`);
    },
  },
});
function Fixture() {
  const [open,setOpen]=useState(true);
  return <ThemeProvider><button id="reopen" onClick={()=>setOpen(true)}>Search</button>
    <FileSpaceSearchPanel open={open} files={files} folders={[]} scopes={["name","content"]}
      onOpen={()=>setOpen(true)} onClose={()=>setOpen(false)} onOpenFile={(file, context)=>{(window as any).__opened={file,context};setOpen(false);}} />
  </ThemeProvider>;
}
createRoot(document.getElementById("root")!).render(<Fixture />);
