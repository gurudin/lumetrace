import { Component, type ComponentType, type ReactNode } from "react";
import type { FileOpenSearchContext } from "../../pages/file-space/fileOpenSearchContext";

/** Trusted application content adapter. This is not an API exposed to downloaded code. */
export interface DocumentViewProps {
  sessionKey: string;
  fileId: string;
  name: string;
  value: string;
  mode: "preview" | "edit";
  readOnly: boolean;
  language: string;
  searchContext: FileOpenSearchContext | null;
  onChange: (value: string) => void;
  fallback: ReactNode;
}

export class DocumentViewSlot extends Component<DocumentViewProps & { view: ComponentType<DocumentViewProps> }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  render() {
    if (this.state.failed) return this.props.fallback;
    const View = this.props.view;
    return <View {...this.props} />;
  }
}
