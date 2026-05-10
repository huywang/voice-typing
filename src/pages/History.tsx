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

      {records.length === 0 && !loading && (
        <p className="empty-hint">No transcription history yet.</p>
      )}

      <div className="history-list">
        {records.map((record) => (
          <div
            key={record.id}
            className={`history-item ${expandedId === record.id ? "expanded" : ""}`}
            onClick={() =>
              setExpandedId(expandedId === record.id ? null : record.id)
            }
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
