import { useState } from "react";
import { ECHO_PRESET, type ChatSettings as Settings } from "../../lib/chat-settings";

/**
 * Chat settings, sitting just above the input. Honest, user-facing DISPLAY toggles
 * (show reasoning, auto-scroll) up front; the developer recipe handle / prompt param
 * sit behind an "advanced (developer)" reveal so a New Chat user sees a clean panel.
 * The model is chosen in the ModelPicker; there is no temperature/top-p (the runtime
 * has no per-turn sampling params today). The single `<summary>` + the directly-
 * visible `echo-preset` button are load-bearing for the chat e2e — do not nest them.
 */
export function ChatSettingsPanel({
  settings,
  onChange,
}: {
  settings: Settings;
  onChange: (s: Settings) => void;
}) {
  const [advanced, setAdvanced] = useState(false);
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
        <button
          type="button"
          className="linkbtn"
          data-testid="echo-preset"
          onClick={() => onChange({ ...settings, ...ECHO_PRESET })}
        >
          Use model-free echo preset
        </button>
        <button
          type="button"
          className="linkbtn chat-settings__advanced-toggle"
          aria-expanded={advanced}
          onClick={() => setAdvanced((a) => !a)}
        >
          {advanced ? "Hide advanced" : "Advanced (developer)"}
        </button>
        {advanced ? (
          <div className="chat-settings__advanced">
            <label htmlFor="chat-handle">Blueprint handle</label>
            <input
              id="chat-handle"
              value={settings.handle}
              onChange={(e) => onChange({ ...settings, handle: e.target.value })}
              spellCheck={false}
              autoComplete="off"
            />
            <label htmlFor="chat-promptkey">Prompt parameter</label>
            <input
              id="chat-promptkey"
              value={settings.promptKey}
              onChange={(e) => onChange({ ...settings, promptKey: e.target.value })}
              spellCheck={false}
              autoComplete="off"
            />
          </div>
        ) : null}
      </div>
    </details>
  );
}
