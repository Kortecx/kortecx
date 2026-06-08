import { ECHO_PRESET, type ChatSettings as Settings } from "../../lib/chat-settings";

/** Chat settings: which recipe backs chat, its prompt param, + display toggles. */
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
      <label htmlFor="chat-handle">Recipe handle</label>
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
      <button
        type="button"
        className="linkbtn"
        data-testid="echo-preset"
        onClick={() => onChange({ ...settings, ...ECHO_PRESET })}
      >
        Use model-free echo preset
      </button>
      <label className="chat-settings__check">
        <input
          type="checkbox"
          checked={settings.showThinking}
          onChange={(e) => onChange({ ...settings, showThinking: e.target.checked })}
        />
        Show DAG-of-thought
      </label>
      <label className="chat-settings__check">
        <input
          type="checkbox"
          checked={settings.autoscroll}
          onChange={(e) => onChange({ ...settings, autoscroll: e.target.checked })}
        />
        Auto-scroll
      </label>
    </details>
  );
}
