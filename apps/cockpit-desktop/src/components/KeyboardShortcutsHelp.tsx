import { X } from "lucide-react";
import { KEYBOARD_SHORTCUTS } from "../config/constants";

interface Props {
  visible: boolean;
  onClose: () => void;
}

export function KeyboardShortcutsHelp({ visible, onClose }: Props) {
  if (!visible) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
      onClick={onClose}
    >
      <div
        className="w-96 border border-zinc-700 bg-zinc-900 p-4"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-3 flex items-center justify-between">
          <h2 className="text-lg font-semibold">Keyboard Shortcuts</h2>
          <button
            aria-label="Close"
            className="control-button h-6 w-6"
            onClick={onClose}
          >
            <X className="h-4 w-4" />
          </button>
        </div>
        <div className="space-y-2 text-sm">
          <div className="flex justify-between border-b border-zinc-800 pb-2">
            <span className="text-zinc-300">Action</span>
            <span className="text-zinc-300">Key</span>
          </div>
          <div className="flex justify-between">
            <span>Pause/Resume</span>
            <kbd className="rounded border border-zinc-700 bg-zinc-800 px-2 py-1 text-xs">
              Space
            </kbd>
          </div>
          <div className="flex justify-between">
            <span>Step Simulation</span>
            <kbd className="rounded border border-zinc-700 bg-zinc-800 px-2 py-1 text-xs">
              S
            </kbd>
          </div>
          <div className="flex justify-between">
            <span>Show This Help</span>
            <kbd className="rounded border border-zinc-700 bg-zinc-800 px-2 py-1 text-xs">
              ?
            </kbd>
          </div>
          <div className="flex justify-between">
            <span>Close Dialog</span>
            <kbd className="rounded border border-zinc-700 bg-zinc-800 px-2 py-1 text-xs">
              Esc
            </kbd>
          </div>
        </div>
      </div>
    </div>
  );
}
