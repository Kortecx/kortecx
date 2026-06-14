import { useState } from "react";
import { toUiError } from "../../kx/errors";
import { useFeedback } from "../../kx/use-feedback";
import type { ChatMessage } from "../../lib/chat-thread";
import { Icon } from "../shell/Icon";

/**
 * 👍/👎 feedback on an assistant answer (PR-4.1). Records into the gateway's
 * `feedback.db` sidecar via `SubmitFeedback`; re-rating the same answer
 * overwrites (server-deterministic id). Optimistic local selection; a gateway
 * without the seam (`not-wired`) HIDES the control — don't-fake-gaps.
 */
export function FeedbackButtons({
  message,
  recipeHandle,
  modelId,
}: {
  message: ChatMessage;
  recipeHandle?: string;
  modelId?: string;
}) {
  const feedback = useFeedback();
  const [selected, setSelected] = useState<"up" | "down" | null>(null);

  // An OLD gateway without the feedback seam degrades to a hidden control.
  if (feedback.isError && toUiError(feedback.error).kind === "not-wired") {
    return null;
  }

  function rate(rating: "up" | "down"): void {
    setSelected(rating);
    feedback.mutate(
      {
        rating,
        messageId: message.id,
        instanceId: message.instanceId,
        moteId: message.terminalMoteId,
        recipeHandle,
        modelId,
      },
      {
        // A transport/other failure (not not-wired) clears the optimistic pick.
        onError: (e) => {
          if (toUiError(e).kind !== "not-wired") {
            setSelected(null);
          }
        },
      },
    );
  }

  return (
    <span className="msg-feedback">
      <button
        type="button"
        className={`msg-action${selected === "up" ? " msg-action--on" : ""}`}
        onClick={() => rate("up")}
        aria-pressed={selected === "up"}
        aria-label="Good answer"
        title="Good answer"
        data-testid="msg-feedback-up"
      >
        <Icon name="thumb-up" />
      </button>
      <button
        type="button"
        className={`msg-action${selected === "down" ? " msg-action--on" : ""}`}
        onClick={() => rate("down")}
        aria-pressed={selected === "down"}
        aria-label="Bad answer"
        title="Bad answer"
        data-testid="msg-feedback-down"
      >
        <Icon name="thumb-down" />
      </button>
    </span>
  );
}
