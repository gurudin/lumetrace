import { HelpCenter } from "./shared/help/HelpCenter";
import { Fragment, useEffect, useLayoutEffect } from "react";
import { ExternalDocumentOpenBridge } from "./pages/file-space/ExternalDocumentOpenBridge";
import { FileSpacePage } from "./pages/file-space/FileSpacePage";
import { ImagePreviewOverlay } from "./pages/file-space/ImagePreviewOverlay";
import { MarkdownPreviewOverlay } from "./pages/file-space/MarkdownPreviewOverlay";
import { PdfPreviewOverlay } from "./pages/file-space/PdfPreviewOverlay";
import { TextPreviewOverlay } from "./pages/file-space/TextPreviewOverlay";
import "./pages/file-space/task-version-timeline-rail.css";
import { ThemeProvider } from "./shared/theme/ThemeProvider";
import { ApplicationExtensionHost, useWorkspaceExtension, type ApplicationExtension } from "./shared/extensions/ApplicationExtension";
import { setWorkspaceCommandSource } from "./shared/extensions/workspaceCommands";

function WorkspaceView() {
  const workspace = useWorkspaceExtension();
  useLayoutEffect(() => setWorkspaceCommandSource(workspace?.source ?? null), [workspace?.source]);
  if (workspace?.initializing) return <section className="file-space-page file-space-page--loading" aria-busy="true"><div className="file-space-loading-state" role="status">LumeTrace</div></section>;
  return <Fragment key={`${workspace?.selectionKey ?? "local"}:${workspace?.source?.key ?? "native"}`}>
    {workspace?.source?.capabilities?.content !== false ? <><PdfPreviewOverlay /><ImagePreviewOverlay /><MarkdownPreviewOverlay /><TextPreviewOverlay /></> : null}
    {!workspace?.active ? <ExternalDocumentOpenBridge /> : null}
    <FileSpacePage />
  </Fragment>;
}

export default function App({ extension }: { extension?: ApplicationExtension }) {
  useEffect(() => {
    const preventFileDropNavigation = (event: DragEvent) => {
      if (!event.dataTransfer || !Array.from(event.dataTransfer.types).includes("Files")) return;
      event.preventDefault();
    };
    window.addEventListener("dragover", preventFileDropNavigation, true);
    window.addEventListener("drop", preventFileDropNavigation, true);
    return () => {
      window.removeEventListener("dragover", preventFileDropNavigation, true);
      window.removeEventListener("drop", preventFileDropNavigation, true);
    };
  }, []);

  return (
    <ThemeProvider>
      <ApplicationExtensionHost extension={extension}>
        <WorkspaceView />
        <HelpCenter />
      </ApplicationExtensionHost>
    </ThemeProvider>
  );
}
