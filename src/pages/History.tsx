import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

interface HistoryRecord {
  id: number;
  timestamp: string;
  raw_text: string;
  polished_text: string;
  duration_secs: number;
  app_name: string;
}

interface HistoryProps {
  onBack: () => void;
}

const PAGE_SIZE = 20;

function History({ onBack }: HistoryProps) {
  const [records, setRecords] = useState<HistoryRecord[]>([]);
  const [totalCount, setTotalCount] = useState(0);
  const [loading, setLoading] = useState(false);
  const [expandedId, setExpandedId] = useState<number | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const [confirmDeleteId, setConfirmDeleteId] = useState<number | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const loadRecords = useCallback(async (offset: number, append: boolean) => {
    setLoading(true);
    try {
      const items = await invoke<HistoryRecord[]>("get_history", {
        limit: PAGE_SIZE,
        offset,
      });
      const count = await invoke<number>("get_history_count");
      setRecords((prev) => (append ? [...prev, ...items] : items));
      setTotalCount(count);
    } catch (e) {
      console.error("Failed to load history:", e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadRecords(0, false);
  }, [loadRecords]);

  const handleLoadMore = () => {
    loadRecords(records.length, true);
  };

  const handleClearAll = async () => {
    if (!confirmClear) {
      setConfirmClear(true);
      return;
    }
    try {
      await invoke("clear_history");
      setRecords([]);
      setTotalCount(0);
      setConfirmClear(false);
    } catch (e) {
      console.error("Failed to clear history:", e);
    }
  };

  /** Copy the polished text of a record to the clipboard. */
  const handleCopy = async (e: React.MouseEvent, record: HistoryRecord) => {
    e.stopPropagation();
    try {
      await navigator.clipboard.writeText(record.polished_text);
    } catch (err) {
      console.error("Failed to copy text:", err);
      setActionError("Failed to copy text to clipboard.");
    }
  };

  /** Re-inject the polished text of a record into the focused input field. */
  const handleReinject = async (e: React.MouseEvent, record: HistoryRecord) => {
    e.stopPropagation();
    try {
      await invoke("reinject_history_item", { id: record.id });
    } catch (err) {
      console.error("Failed to re-inject:", err);
      setActionError(`Failed to re-inject: ${err}`);
    }
  };

  /** Request delete confirmation; on second click, execute delete and refresh. */
  const handleDelete = async (e: React.MouseEvent, record: HistoryRecord) => {
    e.stopPropagation();
    if (confirmDeleteId !== record.id) {
      setConfirmDeleteId(record.id);
      return;
    }
    try {
      await invoke("delete_history_item", { id: record.id });
      setConfirmDeleteId(null);
      // Remove the item from local state and refresh the count.
      setRecords((prev) => prev.filter((r) => r.id !== record.id));
      setTotalCount((prev) => Math.max(0, prev - 1));
      if (expandedId === record.id) setExpandedId(null);
    } catch (err) {
      console.error("Failed to delete history item:", err);
      setActionError(`Failed to delete: ${err}`);
    }
  };

  const formatTimestamp = (ts: string) => {
    const date = new Date(ts);
    return date.toLocaleString();
  };

  const formatDuration = (secs: number) => {
    if (secs < 60) return `${secs.toFixed(1)}s`;
    const mins = Math.floor(secs / 60);
    const remainSecs = secs % 60;
    return `${mins}m ${remainSecs.toFixed(0)}s`;
  };

  return (
    <main className="container history">
      <h1>History</h1>

      <div className="actions">
        <button onClick={onBack}>Back</button>
        {records.length > 0 && (
          <button
            className={confirmClear ? "danger" : ""}
            onClick={handleClearAll}
          >
            {confirmClear ? "Confirm Clear All?" : "Clear All"}
          </button>
        )}
        {confirmClear && (
          <button onClick={() => setConfirmClear(false)}>Cancel</button>
        )}
      </div>

      {actionError && (
        <p className="error-hint" onClick={() => setActionError(null)}>
          {actionError}
        </p>
      )}

      {records.length === 0 && !loading && (
        <p className="empty-hint">
          No transcription history yet.
          <br />
          Use the hotkey to start your first voice input.
        </p>
      )}

      <div className="history-list">
        {records.map((record) => (
          <div
            key={record.id}
            className={`history-item ${expandedId === record.id ? "expanded" : ""}`}
            onClick={() => {
              setExpandedId(expandedId === record.id ? null : record.id);
              // Reset per-item confirm state when collapsing.
              if (confirmDeleteId === record.id) setConfirmDeleteId(null);
            }}
          >
            <div className="history-item-header">
              <span className="history-timestamp">
                {formatTimestamp(record.timestamp)}
              </span>
              <span className="history-duration">
                {formatDuration(record.duration_secs)}
              </span>
            </div>
            <p className="history-preview">
              {record.polished_text.length > 100
                ? record.polished_text.slice(0, 100) + "..."
                : record.polished_text}
            </p>
            {expandedId === record.id && (
              <div className="history-detail">
                <div className="detail-section">
                  <span className="detail-label">Polished text:</span>
                  <p className="detail-text">{record.polished_text}</p>
                </div>
                {record.raw_text !== record.polished_text && (
                  <div className="detail-section">
                    <span className="detail-label">Raw ASR text:</span>
                    <p className="detail-text">{record.raw_text}</p>
                  </div>
                )}
                {/* Per-item action buttons */}
                <div className="history-item-actions" onClick={(e) => e.stopPropagation()}>
                  <button
                    className="item-action"
                    onClick={(e) => handleCopy(e, record)}
                  >
                    Copy
                  </button>
                  <button
                    className="item-action"
                    onClick={(e) => handleReinject(e, record)}
                  >
                    Re-inject
                  </button>
                  <button
                    className={`item-action danger ${confirmDeleteId === record.id ? "active" : ""}`}
                    onClick={(e) => handleDelete(e, record)}
                  >
                    {confirmDeleteId === record.id ? "Confirm Delete?" : "Delete"}
                  </button>
                  {confirmDeleteId === record.id && (
                    <button
                      className="item-action"
                      onClick={(e) => {
                        e.stopPropagation();
                        setConfirmDeleteId(null);
                      }}
                    >
                      Cancel
                    </button>
                  )}
                </div>
              </div>
            )}
          </div>
        ))}
      </div>

      {records.length < totalCount && (
        <div className="actions">
          <button onClick={handleLoadMore} disabled={loading}>
            {loading ? "Loading..." : `Load More (${totalCount - records.length} remaining)`}
          </button>
        </div>
      )}

      {loading && records.length === 0 && <p className="empty-hint">Loading...</p>}
    </main>
  );
}

export default History;
