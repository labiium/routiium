import { useCallback, useEffect, useState } from "react";
import { ErrorState, LoadingState } from "../../components/AsyncState";
import { fetchJson, sendJson } from "../../lib/adminApi";
import { formatDateTime, formatJson, formatNumber } from "../../lib/formatters";
import { useAdminPanelState } from "../../hooks/useAdminPanelState";

function ChatHistory() {
    const { data, error: panelError, isLoading: panelLoading, refresh: refreshPanel } =
        useAdminPanelState();
    const [conversations, setConversations] = useState<any[]>([]);
    const [messages, setMessages] = useState<any[]>([]);
    const [selectedConversationId, setSelectedConversationId] = useState<string | null>(null);
    const [isLoading, setIsLoading] = useState(true);
    const [error, setError] = useState<string | null>(null);

    const loadConversations = useCallback(async () => {
        try {
            setError(null);
            setIsLoading(true);
            const payload = await fetchJson<any>("/chat_history/conversations?limit=100");
            setConversations(payload.conversations || []);
        } catch (err) {
            setError(err instanceof Error ? err.message : "Failed to load chat history");
        } finally {
            setIsLoading(false);
        }
    }, []);

    const loadMessages = useCallback(async (conversationId: string) => {
        const payload = await fetchJson<any>(
            `/chat_history/messages?conversation_id=${encodeURIComponent(conversationId)}&limit=200`,
        );
        setMessages(payload.messages || []);
        setSelectedConversationId(conversationId);
    }, []);

    useEffect(() => {
        loadConversations();
    }, [loadConversations]);

    if (panelLoading || isLoading) {
        return (
            <LoadingState
                title="Chat History"
                description="Stored conversations and messages captured by Routiium."
            />
        );
    }

    if (panelError || error || !data) {
        return (
            <ErrorState
                title="Chat History"
                description="Stored conversations and messages captured by Routiium."
                error={panelError || error}
                onRetry={async () => {
                    await refreshPanel();
                    await loadConversations();
                }}
            />
        );
    }

    const chatHistoryOverview = data.overview.chat_history || {};
    const chatHistorySettings = data.settings.chat_history || {};

    return (
        <div>
            <div className="page-header">
                <div className="page-header-row">
                    <div>
                        <h1 className="page-title">Chat History</h1>
                        <p className="page-description">
                            Browse stored conversations, inspect messages, and clear or delete
                            captured history.
                        </p>
                    </div>
                    <div className="page-actions">
                        <button
                            className="btn btn-secondary"
                            onClick={async () => {
                                await refreshPanel();
                                await loadConversations();
                            }}
                        >
                            Refresh
                        </button>
                        <button
                            className="btn btn-ghost"
                            onClick={async () => {
                                await sendJson("/chat_history/clear", "POST");
                                setMessages([]);
                                setSelectedConversationId(null);
                                await refreshPanel();
                                await loadConversations();
                            }}
                        >
                            Clear All
                        </button>
                    </div>
                </div>
            </div>

            <div className="stats-grid">
                <div className="stat-card">
                    <div className="stat-label">Backend</div>
                    <div className="stat-value">
                        {chatHistoryOverview.stats?.backend_type || chatHistorySettings.primary_backend}
                    </div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Conversations</div>
                    <div className="stat-value">
                        {formatNumber(chatHistoryOverview.stats?.total_conversations)}
                    </div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Messages</div>
                    <div className="stat-value">
                        {formatNumber(chatHistoryOverview.stats?.total_messages)}
                    </div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Privacy</div>
                    <div className="stat-value">{chatHistorySettings.privacy_level}</div>
                </div>
            </div>

            <div className="section" style={{ display: "grid", gap: "var(--space-6)" }}>
                <div className="card">
                    <div className="card-header">
                        <div>
                            <h3 className="card-title">Conversations</h3>
                            <p className="card-description">Recent stored conversations.</p>
                        </div>
                    </div>
                    <div className="card-content">
                        <div className="table-container">
                            <table className="table">
                                <thead>
                                    <tr>
                                        <th>Conversation</th>
                                        <th>Created</th>
                                        <th>Last Seen</th>
                                        <th></th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {conversations.map((conversation) => (
                                        <tr key={conversation.conversation_id}>
                                            <td>{conversation.title || conversation.conversation_id}</td>
                                            <td>{formatDateTime(conversation.created_at)}</td>
                                            <td>{formatDateTime(conversation.last_seen_at)}</td>
                                            <td style={{ display: "flex", gap: "var(--space-2)" }}>
                                                <button
                                                    className="btn btn-secondary"
                                                    onClick={() =>
                                                        loadMessages(conversation.conversation_id)
                                                    }
                                                >
                                                    Inspect
                                                </button>
                                                <button
                                                    className="btn btn-ghost"
                                                    onClick={async () => {
                                                        await sendJson(
                                                            `/chat_history/conversations/${conversation.conversation_id}`,
                                                            "DELETE",
                                                        );
                                                        if (
                                                            selectedConversationId ===
                                                            conversation.conversation_id
                                                        ) {
                                                            setSelectedConversationId(null);
                                                            setMessages([]);
                                                        }
                                                        await refreshPanel();
                                                        await loadConversations();
                                                    }}
                                                >
                                                    Delete
                                                </button>
                                            </td>
                                        </tr>
                                    ))}
                                    {conversations.length === 0 ? (
                                        <tr>
                                            <td colSpan={4}>No conversations stored.</td>
                                        </tr>
                                    ) : null}
                                </tbody>
                            </table>
                        </div>
                    </div>
                </div>

                <div className="card">
                    <div className="card-header">
                        <div>
                            <h3 className="card-title">Messages</h3>
                            <p className="card-description">
                                {selectedConversationId
                                    ? `Messages for ${selectedConversationId}`
                                    : "Select a conversation to inspect messages."}
                            </p>
                        </div>
                    </div>
                    <div className="card-content">
                        {messages.length === 0 ? (
                            <div>No messages loaded.</div>
                        ) : (
                            <div style={{ display: "grid", gap: "var(--space-4)" }}>
                                {messages.map((message) => (
                                    <div key={message.message_id} className="card">
                                        <div className="card-content">
                                            <div
                                                style={{
                                                    display: "flex",
                                                    justifyContent: "space-between",
                                                    marginBottom: "var(--space-2)",
                                                }}
                                            >
                                                <strong>{message.role}</strong>
                                                <span>{formatDateTime(message.created_at)}</span>
                                            </div>
                                            <pre
                                                style={{
                                                    margin: 0,
                                                    whiteSpace: "pre-wrap",
                                                    fontFamily: "monospace",
                                                    fontSize: "0.9rem",
                                                }}
                                            >
                                                {formatJson(message.content)}
                                            </pre>
                                        </div>
                                    </div>
                                ))}
                            </div>
                        )}
                    </div>
                </div>
            </div>
        </div>
    );
}

export default ChatHistory;
