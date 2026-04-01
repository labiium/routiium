import JsonEditorCard from "../components/JsonEditorCard";
import { ErrorState, LoadingState } from "../components/AsyncState";
import { sendJson } from "../lib/adminApi";
import { formatNumber } from "../lib/formatters";
import { useAdminPanelState } from "../hooks/useAdminPanelState";

function Routing() {
    const { data, error, isLoading, refresh } = useAdminPanelState();

    if (isLoading) {
        return (
            <LoadingState
                title="Routing"
                description="Inspect local routing config and router catalog state."
            />
        );
    }

    if (error || !data) {
        return (
            <ErrorState
                title="Routing"
                description="Inspect local routing config and router catalog state."
                error={error}
                onRetry={refresh}
            />
        );
    }

    const routing = data.routing;
    const stats = routing.stats || {};
    const catalogModels = routing.router?.catalog_models || [];

    return (
        <div>
            <div className="page-header">
                <div className="page-header-row">
                    <div>
                        <h1 className="page-title">Routing</h1>
                        <p className="page-description">
                            Manage the local `routing.json` rules and inspect any remote/local
                            router catalog Routiium is using.
                        </p>
                    </div>
                    <div className="page-actions">
                        <button
                            className="btn btn-secondary"
                            onClick={async () => {
                                await sendJson("/reload/routing", "POST");
                                await refresh();
                            }}
                            disabled={!routing.config_path}
                        >
                            Reload From Disk
                        </button>
                    </div>
                </div>
            </div>

            <div className="stats-grid">
                <div className="stat-card">
                    <div className="stat-label">Router Mode</div>
                    <div className="stat-value">{routing.router?.mode || "none"}</div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Rules</div>
                    <div className="stat-value">{formatNumber(stats.total_rules)}</div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Aliases</div>
                    <div className="stat-value">{formatNumber(stats.total_aliases)}</div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Catalog Models</div>
                    <div className="stat-value">{formatNumber(catalogModels.length)}</div>
                </div>
            </div>

            <div className="section">
                <div className="card">
                    <div className="card-header">
                        <div>
                            <h3 className="card-title">Router Catalog</h3>
                            <p className="card-description">
                                Models published by the configured router, if available.
                            </p>
                        </div>
                    </div>
                    <div className="card-content">
                        <div className="table-container">
                            <table className="table">
                                <thead>
                                    <tr>
                                        <th>Model</th>
                                        <th>Provider</th>
                                        <th>Status</th>
                                        <th>Aliases</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {catalogModels.map((model: any) => (
                                        <tr key={model.id}>
                                            <td>{model.id}</td>
                                            <td>{model.provider}</td>
                                            <td>{model.status}</td>
                                            <td>{(model.aliases || []).join(", ") || "—"}</td>
                                        </tr>
                                    ))}
                                    {catalogModels.length === 0 ? (
                                        <tr>
                                            <td colSpan={4}>No router catalog is available.</td>
                                        </tr>
                                    ) : null}
                                </tbody>
                            </table>
                        </div>
                    </div>
                </div>
            </div>

            <JsonEditorCard
                title="Local Routing Config"
                description={
                    routing.config_path
                        ? `Backed by ${routing.config_path}`
                        : "This Routiium instance is not using a file-backed local routing config."
                }
                value={routing.config}
                disabled={!routing.config_path}
                onSave={async (value) => {
                    await sendJson("/admin/panel/routing", "PUT", value);
                    await refresh();
                }}
            />
        </div>
    );
}

export default Routing;
