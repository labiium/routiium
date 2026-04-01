import JsonEditorCard from "../../components/JsonEditorCard";
import { ErrorState, LoadingState } from "../../components/AsyncState";
import { sendJson } from "../../lib/adminApi";
import { useAdminPanelState } from "../../hooks/useAdminPanelState";

function Mcp() {
    const { data, error, isLoading, refresh } = useAdminPanelState();

    if (isLoading) {
        return (
            <LoadingState
                title="MCP"
                description="Connected Model Context Protocol servers and tools."
            />
        );
    }

    if (error || !data) {
        return (
            <ErrorState
                title="MCP"
                description="Connected Model Context Protocol servers and tools."
                error={error}
                onRetry={refresh}
            />
        );
    }

    const mcp = data.mcp;
    const tools = mcp.tools || [];

    return (
        <div>
            <div className="page-header">
                <div className="page-header-row">
                    <div>
                        <h1 className="page-title">MCP</h1>
                        <p className="page-description">
                            Edit the `mcp.json` server config and inspect the tools Routiium has
                            connected successfully.
                        </p>
                    </div>
                    <div className="page-actions">
                        <button
                            className="btn btn-secondary"
                            onClick={async () => {
                                await sendJson("/reload/mcp", "POST");
                                await refresh();
                            }}
                            disabled={!mcp.config_path}
                        >
                            Reload From Disk
                        </button>
                    </div>
                </div>
            </div>

            <div className="stats-grid">
                <div className="stat-card">
                    <div className="stat-label">Configured Servers</div>
                    <div className="stat-value">
                        {(mcp.configured_servers || []).length}
                    </div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Connected Servers</div>
                    <div className="stat-value">
                        {(mcp.connected_servers || []).length}
                    </div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Tools</div>
                    <div className="stat-value">{tools.length}</div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Config Backing</div>
                    <div className="stat-value">{mcp.config_path ? "File" : "None"}</div>
                </div>
            </div>

            <div className="section">
                <div className="card">
                    <div className="card-header">
                        <div>
                            <h3 className="card-title">Connected Tools</h3>
                            <p className="card-description">
                                Tools exported by connected MCP servers.
                            </p>
                        </div>
                    </div>
                    <div className="card-content">
                        <div className="table-container">
                            <table className="table">
                                <thead>
                                    <tr>
                                        <th>Server</th>
                                        <th>Tool</th>
                                        <th>Combined Name</th>
                                        <th>Description</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {tools.map((tool: any) => (
                                        <tr key={tool.combined_name}>
                                            <td>{tool.server_name}</td>
                                            <td>{tool.name}</td>
                                            <td>{tool.combined_name}</td>
                                            <td>{tool.description || "—"}</td>
                                        </tr>
                                    ))}
                                    {tools.length === 0 ? (
                                        <tr>
                                            <td colSpan={4}>No MCP tools are currently connected.</td>
                                        </tr>
                                    ) : null}
                                </tbody>
                            </table>
                        </div>
                    </div>
                </div>
            </div>

            <JsonEditorCard
                title="MCP Config"
                description={
                    mcp.config_path
                        ? `Backed by ${mcp.config_path}`
                        : "This Routiium instance did not load a file-backed MCP config."
                }
                value={mcp.config}
                disabled={!mcp.config_path}
                onSave={async (value) => {
                    await sendJson("/admin/panel/mcp", "PUT", value);
                    await refresh();
                }}
            />
        </div>
    );
}

export default Mcp;
