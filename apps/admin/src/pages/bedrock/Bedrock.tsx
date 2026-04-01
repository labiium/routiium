import { ErrorState, LoadingState } from "../../components/AsyncState";
import { useAdminPanelState } from "../../hooks/useAdminPanelState";

function Bedrock() {
    const { data, error, isLoading, refresh } = useAdminPanelState();

    if (isLoading) {
        return (
            <LoadingState
                title="Bedrock"
                description="Detected AWS Bedrock posture from runtime config and router state."
            />
        );
    }

    if (error || !data) {
        return (
            <ErrorState
                title="Bedrock"
                description="Detected AWS Bedrock posture from runtime config and router state."
                error={error}
                onRetry={refresh}
            />
        );
    }

    const bedrock = data.bedrock;
    const routingBackends = bedrock.routing_backends || [];
    const catalogModels = bedrock.router_catalog_models || [];

    return (
        <div>
            <div className="page-header">
                <div className="page-header-row">
                    <div>
                        <h1 className="page-title">Bedrock</h1>
                        <p className="page-description">
                            Read-only Bedrock detection across direct upstream mode, local
                            routing backends, and router catalog models.
                        </p>
                    </div>
                    <div className="page-actions">
                        <button className="btn btn-secondary" onClick={refresh}>
                            Refresh
                        </button>
                    </div>
                </div>
            </div>

            <div className="stats-grid">
                <div className="stat-card">
                    <div className="stat-label">Detected</div>
                    <div className="stat-value">{bedrock.enabled ? "Yes" : "No"}</div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Default Upstream</div>
                    <div className="stat-value">{bedrock.default_upstream_mode}</div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">AWS Region</div>
                    <div className="stat-value">{bedrock.aws_region || "—"}</div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Credentials Source</div>
                    <div className="stat-value">{bedrock.credentials_source}</div>
                </div>
            </div>

            <div className="section">
                <div className="card">
                    <div className="card-header">
                        <div>
                            <h3 className="card-title">Routing Backends</h3>
                            <p className="card-description">
                                Local routing rules that route to Bedrock-compatible upstreams.
                            </p>
                        </div>
                    </div>
                    <div className="card-content">
                        <div className="table-container">
                            <table className="table">
                                <thead>
                                    <tr>
                                        <th>Rule</th>
                                        <th>Description</th>
                                        <th>Base URL</th>
                                        <th>Key Env</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {routingBackends.map((backend: any, index: number) => (
                                        <tr key={`${backend.rule_id || "default"}-${index}`}>
                                            <td>{backend.rule_id || "default_backend"}</td>
                                            <td>{backend.description || "—"}</td>
                                            <td>{backend.base_url}</td>
                                            <td>{backend.key_env || "—"}</td>
                                        </tr>
                                    ))}
                                    {routingBackends.length === 0 ? (
                                        <tr>
                                            <td colSpan={4}>No Bedrock routing backends detected.</td>
                                        </tr>
                                    ) : null}
                                </tbody>
                            </table>
                        </div>
                    </div>
                </div>
            </div>

            <div className="section">
                <div className="card">
                    <div className="card-header">
                        <div>
                            <h3 className="card-title">Router Catalog Models</h3>
                            <p className="card-description">
                                Bedrock models exposed by the configured router catalog.
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
                                        <th>Regions</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {catalogModels.map((model: any) => (
                                        <tr key={model.id}>
                                            <td>{model.id}</td>
                                            <td>{model.provider}</td>
                                            <td>{model.status}</td>
                                            <td>{(model.region || []).join(", ") || "—"}</td>
                                        </tr>
                                    ))}
                                    {catalogModels.length === 0 ? (
                                        <tr>
                                            <td colSpan={4}>No Bedrock catalog models detected.</td>
                                        </tr>
                                    ) : null}
                                </tbody>
                            </table>
                        </div>
                    </div>
                </div>
            </div>
        </div>
    );
}

export default Bedrock;
