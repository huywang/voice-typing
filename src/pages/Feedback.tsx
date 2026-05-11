import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface FeedbackProps {
  onBack: () => void;
}

type FeedbackType = "feature" | "bug";
type SubmitState = "idle" | "submitting" | "success" | "error";

function Feedback({ onBack }: FeedbackProps) {
  const [feedbackType, setFeedbackType] = useState<FeedbackType>("feature");
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [submitState, setSubmitState] = useState<SubmitState>("idle");
  const [issueUrl, setIssueUrl] = useState("");
  const [errorMessage, setErrorMessage] = useState("");

  const handleSubmit = async () => {
    const trimmedTitle = title.trim();
    if (!trimmedTitle) return;

    setSubmitState("submitting");
    setErrorMessage("");

    try {
      const url = await invoke<string>("submit_feedback", {
        feedbackType,
        title: trimmedTitle,
        description: description.trim(),
      });
      setIssueUrl(url);
      setSubmitState("success");
    } catch (e) {
      setErrorMessage(String(e));
      setSubmitState("error");
    }
  };

  const handleReset = () => {
    setTitle("");
    setDescription("");
    setFeedbackType("feature");
    setSubmitState("idle");
    setIssueUrl("");
    setErrorMessage("");
  };

  return (
    <main className="container settings">
      <h1>Feedback</h1>

      {submitState === "success" ? (
        <div className="feedback-success">
          <p className="feedback-success-icon">✓</p>
          <p className="feedback-success-title">Submitted!</p>
          <p className="feedback-success-desc">
            Your feedback has been created as a GitHub issue.
          </p>
          {issueUrl && (
            <a
              className="feedback-issue-link"
              href={issueUrl}
              target="_blank"
            >
              View issue on GitHub
            </a>
          )}
          <div className="actions" style={{ marginTop: "16px" }}>
            <button onClick={handleReset}>Submit Another</button>
            <button onClick={onBack}>Done</button>
          </div>
        </div>
      ) : (
        <div className="settings-tabs">
          <div className="tab-panel" role="form" style={{ overflow: "visible" }}>
            {/* Type selector */}
            <div className="form-group">
              <label>Type</label>
              <div className="feedback-type-selector">
                <button
                  type="button"
                  className={`feedback-type-btn${feedbackType === "feature" ? " active" : ""}`}
                  onClick={() => setFeedbackType("feature")}
                >
                  Feature Request
                </button>
                <button
                  type="button"
                  className={`feedback-type-btn${feedbackType === "bug" ? " active" : ""}`}
                  onClick={() => setFeedbackType("bug")}
                >
                  Bug Report
                </button>
              </div>
            </div>

            {/* Title */}
            <div className="form-group">
              <label htmlFor="feedback-title">Title</label>
              <input
                id="feedback-title"
                type="text"
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                placeholder={
                  feedbackType === "feature"
                    ? "e.g. Support for custom wake words"
                    : "e.g. App crashes when recording is stopped quickly"
                }
                disabled={submitState === "submitting"}
              />
            </div>

            {/* Description */}
            <div className="form-group">
              <label htmlFor="feedback-description">Description</label>
              <textarea
                id="feedback-description"
                className="feedback-textarea"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder={
                  feedbackType === "feature"
                    ? "Describe the feature and why it would be useful..."
                    : "Steps to reproduce, expected vs actual behavior..."
                }
                disabled={submitState === "submitting"}
                rows={5}
              />
            </div>

            {/* Error message */}
            {submitState === "error" && errorMessage && (
              <div className="form-group">
                <p
                  className="form-hint"
                  style={{ color: "var(--system-red)" }}
                  aria-live="polite"
                >
                  {errorMessage}
                </p>
              </div>
            )}
          </div>

          <div className="actions">
            <button onClick={onBack} disabled={submitState === "submitting"}>
              Back
            </button>
            <button
              className="primary"
              onClick={handleSubmit}
              disabled={!title.trim() || submitState === "submitting"}
            >
              {submitState === "submitting" ? "Submitting..." : "Submit"}
            </button>
          </div>
        </div>
      )}
    </main>
  );
}

export default Feedback;
