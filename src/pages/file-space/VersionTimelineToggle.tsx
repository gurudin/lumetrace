import { PanelLeft } from "lucide-react";
import type { ReactNode } from "react";
import "./version-timeline-toggle.css";

interface VersionTimelineRegionProps {
  visible: boolean;
  children: ReactNode;
}

export function VersionTimelineRegion({ visible, children }: VersionTimelineRegionProps) {
  return (
    <div className="file-preview-version-region" aria-hidden={!visible} inert={!visible}>
      <div className="file-preview-version-region-content">{children}</div>
    </div>
  );
}

interface VersionTimelineToggleProps {
  controlsId: string;
  visible: boolean;
  showLabel: string;
  hideLabel: string;
  onToggle: () => void;
}

export function VersionTimelineToggle({
  controlsId,
  visible,
  showLabel,
  hideLabel,
  onToggle,
}: VersionTimelineToggleProps) {
  const label = visible ? hideLabel : showLabel;
  return (
    <button
      className={`file-preview-version-toggle${visible ? " is-active" : ""}`}
      type="button"
      aria-controls={controlsId}
      aria-expanded={visible}
      aria-label={label}
      title={label}
      onPointerDown={(event) => event.stopPropagation()}
      onClick={onToggle}
    >
      <PanelLeft size={18} strokeWidth={1.8} />
    </button>
  );
}
