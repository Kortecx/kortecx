import type { ChatSettings as Settings } from "../../lib/chat-settings";

/**
 * Chat settings, sitting just above the input. A CLEAN panel — just the two
 * honest DISPLAY toggles (show reasoning, auto-scroll). The old model-free `echo`
 * preset button + the "Advanced (developer)" recipe-handle reveal are gone (New Chat
 * is a chat, not a recipe console; the `echo` backing is still selectable via the
 * persisted setting — the e2e seeds it). The model is chosen in the ModelPicker; there
 * is no temperature/top-p (the runtime has no per-turn sampling params today).
 */
export function ChatSettingsPanel({
  settings,
  onChange,
}: {
  settings: Settings;
  onChange: (s: Settings) => void;
}) {
  return (
    <details className="chat-settings" data-testid="chat-settings">
      <summary>Settings</summary>
      <div className="chat-settings__body">
        <label className="chat-settings__check">
          <input
            type="checkbox"
            checked={settings.showReasoning}
            onChange={(e) => onChange({ ...settings, showReasoning: e.target.checked })}
          />
          Show reasoning
        </label>
        <label className="chat-settings__check">
          <input
            type="checkbox"
            checked={settings.autoscroll}
            onChange={(e) => onChange({ ...settings, autoscroll: e.target.checked })}
          />
          Auto-scroll
        </label>
      </div>
    </details>
  );
}
