import JsonEditorCard from "../components/JsonEditorCard";
import { ErrorState, LoadingState } from "../components/AsyncState";
import { sendJson } from "../lib/adminApi";
import { useAdminPanelState } from "../hooks/useAdminPanelState";

function SystemPrompts() {
    const { data, error, isLoading, refresh } = useAdminPanelState();

    if (isLoading) {
        return (
            <LoadingState
                title="System Prompts"
                description="Manage Routiium prompt injection rules."
            />
        );
    }

    if (error || !data) {
        return (
            <ErrorState
                title="System Prompts"
                description="Manage Routiium prompt injection rules."
                error={error}
                onRetry={refresh}
            />
        );
    }

    const systemPrompt = data.system_prompt;

    return (
        <div>
            <div className="page-header">
                <div className="page-header-row">
                    <div>
                        <h1 className="page-title">System Prompts</h1>
                        <p className="page-description">
                            Edit the live prompt injection configuration Routiium uses for
                            `chat`, `responses`, and per-model overrides.
                        </p>
                    </div>
                    <div className="page-actions">
                        <button
                            className="btn btn-secondary"
                            onClick={async () => {
                                await sendJson("/reload/system_prompt", "POST");
                                await refresh();
                            }}
                            disabled={!systemPrompt.config_path}
                        >
                            Reload From Disk
                        </button>
                    </div>
                </div>
            </div>

            <div className="stats-grid">
                <div className="stat-card">
                    <div className="stat-label">Enabled</div>
                    <div className="stat-value">
                        {systemPrompt.summary?.enabled ? "Yes" : "No"}
                    </div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Global Prompt</div>
                    <div className="stat-value">
                        {systemPrompt.summary?.global_configured ? "Configured" : "None"}
                    </div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Per Model Rules</div>
                    <div className="stat-value">
                        {systemPrompt.summary?.per_model_count || 0}
                    </div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Per API Rules</div>
                    <div className="stat-value">
                        {systemPrompt.summary?.per_api_count || 0}
                    </div>
                </div>
            </div>

            <JsonEditorCard
                title="Prompt Configuration"
                description={
                    systemPrompt.config_path
                        ? `Backed by ${systemPrompt.config_path}`
                        : "This Routiium instance did not load a file-backed system prompt config."
                }
                value={systemPrompt.config}
                disabled={!systemPrompt.config_path}
                onSave={async (value) => {
                    await sendJson("/admin/panel/system-prompts", "PUT", value);
                    await refresh();
                }}
            />
        </div>
    );
}

export default SystemPrompts;
