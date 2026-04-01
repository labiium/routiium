import { ErrorState, LoadingState } from "../components/AsyncState";
import { formatJson } from "../lib/formatters";
import { useAdminPanelState } from "../hooks/useAdminPanelState";

function SettingsSection({ title, value }: { title: string; value: unknown }) {
    return (
        <div className="card">
            <div className="card-header">
                <div>
                    <h3 className="card-title">{title}</h3>
                </div>
            </div>
            <div className="card-content">
                <pre
                    style={{
                        margin: 0,
                        whiteSpace: "pre-wrap",
                        fontFamily: "monospace",
                        fontSize: "0.9rem",
                    }}
                >
                    {formatJson(value)}
                </pre>
            </div>
        </div>
    );
}

function Settings() {
    const { data, error, isLoading, refresh } = useAdminPanelState();

    if (isLoading) {
        return (
            <LoadingState
                title="Settings"
                description="Read-only runtime settings exported by Routiium."
            />
        );
    }

    if (error || !data) {
        return (
            <ErrorState
                title="Settings"
                description="Read-only runtime settings exported by Routiium."
                error={error}
                onRetry={refresh}
            />
        );
    }

    const settings = data.settings;

    return (
        <div>
            <div className="page-header">
                <div className="page-header-row">
                    <div>
                        <h1 className="page-title">Settings</h1>
                        <p className="page-description">
                            Routiium’s server, auth, analytics, chat-history, and rate-limit
                            runtime configuration. This panel is intentionally read-only for
                            env-derived settings.
                        </p>
                    </div>
                    <div className="page-actions">
                        <button className="btn btn-secondary" onClick={refresh}>
                            Refresh
                        </button>
                    </div>
                </div>
            </div>

            <div className="section" style={{ display: "grid", gap: "var(--space-6)" }}>
                <SettingsSection title="Auth" value={settings.auth} />
                <SettingsSection title="Server" value={settings.server} />
                <SettingsSection title="CORS" value={settings.cors} />
                <SettingsSection title="Analytics" value={settings.analytics} />
                <SettingsSection title="Chat History" value={settings.chat_history} />
                <SettingsSection title="Rate Limits" value={settings.rate_limits} />
            </div>
        </div>
    );
}

export default Settings;
