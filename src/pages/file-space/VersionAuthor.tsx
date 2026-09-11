import { useTranslation } from "react-i18next";

/** Names are presentation metadata; absent/legacy authors keep the existing label. */
export function VersionAuthor({ name }: { name?: string | null }) {
  const { t } = useTranslation();
  return <>{name?.trim()
    ? t("fileSpace.timeline.editedBy", { name: name.trim() })
    : t("fileSpace.timeline.userEdit")}</>;
}
