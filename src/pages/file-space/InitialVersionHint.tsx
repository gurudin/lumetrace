import { useTranslation } from "react-i18next";
import "./initial-version-hint.css";

export function VersionName({ number }: { number: number }) {
  const { t } = useTranslation();
  return <>v{number}{number === 1 ? ` · ${t("fileSpace.timeline.initialVersion")}` : ""}</>;
}

export function InitialVersionHint({ versions }: { versions: readonly { versionNumber: number }[] | null }) {
  const { t } = useTranslation();
  if (versions?.length !== 1 || versions[0].versionNumber !== 1) return null;
  return <p className="file-initial-version-hint">{t("fileSpace.timeline.initialVersionHint")}</p>;
}
