import { Icon } from "@/components/Icon";
import { useT } from "@/lib/i18n";
import { useTheme } from "@/lib/theme";

export function ThemeButton() {
  const { theme, toggle } = useTheme();
  const t = useT();
  const label = theme === "dark" ? t("theme.toLight") : t("theme.toDark");
  return (
    <button type="button" className="icon-btn" aria-label={label} title={label} onClick={toggle}>
      <Icon name={theme === "dark" ? "sun" : "moon"} size={20} />
    </button>
  );
}
