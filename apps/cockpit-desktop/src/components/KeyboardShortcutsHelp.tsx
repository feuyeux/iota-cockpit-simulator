import { X } from "lucide-react";
import { useI18n } from "../i18n";

interface Props {
  visible: boolean;
  onClose: () => void;
}

export function KeyboardShortcutsHelp({ visible, onClose }: Props) {
  const { t } = useI18n();
  if (!visible) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
      onClick={onClose}
    >
      <div
        className="w-[calc(100%-2rem)] max-w-96 border border-zinc-700 bg-zinc-900 p-4"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-3 flex items-center justify-between">
          <h2 className="text-lg font-semibold">{t("keyboardShortcuts")}</h2>
          <button
            aria-label={t("close")}
            className="control-button h-[26px] w-[26px]"
            onClick={onClose}
          >
            <X className="h-4 w-4" />
          </button>
        </div>
        <div className="space-y-2 text-sm">
          <div className="flex justify-between border-b border-zinc-800 pb-2">
            <span className="text-zinc-300">{t("action")}</span>
            <span className="text-zinc-300">{t("key")}</span>
          </div>
          <div className="flex justify-between">
            <span>{t("pauseResume")}</span>
            <kbd className="rounded border border-zinc-700 bg-zinc-800 px-2 py-1 text-xs">
              Space
            </kbd>
          </div>
          <div className="flex justify-between">
            <span>{t("stepSimulation")}</span>
            <kbd className="rounded border border-zinc-700 bg-zinc-800 px-2 py-1 text-xs">
              S
            </kbd>
          </div>
          <div className="flex justify-between">
            <span>{t("showHelp")}</span>
            <kbd className="rounded border border-zinc-700 bg-zinc-800 px-2 py-1 text-xs">
              ?
            </kbd>
          </div>
          <div className="flex justify-between">
            <span>{t("closeDialog")}</span>
            <kbd className="rounded border border-zinc-700 bg-zinc-800 px-2 py-1 text-xs">
              Esc
            </kbd>
          </div>
        </div>
      </div>
    </div>
  );
}
