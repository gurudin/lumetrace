import { useEffect } from "react";
import { ExternalDocumentOpenBridge } from "./pages/file-space/ExternalDocumentOpenBridge";
import { FileSpacePage } from "./pages/file-space/FileSpacePage";
import { ImagePreviewOverlay } from "./pages/file-space/ImagePreviewOverlay";
import { MarkdownPreviewOverlay } from "./pages/file-space/MarkdownPreviewOverlay";
import { PdfPreviewOverlay } from "./pages/file-space/PdfPreviewOverlay";
import { TextPreviewOverlay } from "./pages/file-space/TextPreviewOverlay";
import "./pages/file-space/task-version-timeline-rail.css";
import { ThemeProvider } from "./shared/theme/ThemeProvider";

export default function App() {
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
      <PdfPreviewOverlay />
      <ImagePreviewOverlay />
      <MarkdownPreviewOverlay />
      <TextPreviewOverlay />
      <ExternalDocumentOpenBridge />
      <FileSpacePage />
    </ThemeProvider>
  );
}
