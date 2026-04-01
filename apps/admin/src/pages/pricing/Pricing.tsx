import { ErrorState, LoadingState } from "../../components/AsyncState";
import { formatCurrency } from "../../lib/formatters";
import { useAdminPanelState } from "../../hooks/useAdminPanelState";

function Pricing() {
    const { data, error, isLoading, refresh } = useAdminPanelState();

    if (isLoading) {
        return (
            <LoadingState
                title="Pricing"
                description="Loaded model pricing used by Routiium cost accounting."
            />
        );
    }

    if (error || !data) {
        return (
            <ErrorState
                title="Pricing"
                description="Loaded model pricing used by Routiium cost accounting."
                error={error}
                onRetry={refresh}
            />
        );
    }

    const pricing = data.pricing;
    const models = pricing.models || [];

    return (
        <div>
            <div className="page-header">
                <div className="page-header-row">
                    <div>
                        <h1 className="page-title">Pricing</h1>
                        <p className="page-description">
                            Routiium pricing source: {pricing.source}.
                            {pricing.config_path ? ` File: ${pricing.config_path}` : ""}
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
                    <div className="stat-label">Source</div>
                    <div className="stat-value">{pricing.source}</div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Models</div>
                    <div className="stat-value">{pricing.models_count}</div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Default Input</div>
                    <div className="stat-value">
                        {formatCurrency(pricing.default_pricing?.input_per_million)}
                    </div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Default Output</div>
                    <div className="stat-value">
                        {formatCurrency(pricing.default_pricing?.output_per_million)}
                    </div>
                </div>
            </div>

            <div className="card">
                <div className="card-content">
                    <div className="table-container">
                        <table className="table">
                            <thead>
                                <tr>
                                    <th>Model</th>
                                    <th>Input / 1M</th>
                                    <th>Output / 1M</th>
                                    <th>Cached / 1M</th>
                                    <th>Reasoning / 1M</th>
                                </tr>
                            </thead>
                            <tbody>
                                {models.map((model: any) => (
                                    <tr key={model.model}>
                                        <td>{model.model}</td>
                                        <td>{formatCurrency(model.input_per_million)}</td>
                                        <td>{formatCurrency(model.output_per_million)}</td>
                                        <td>{formatCurrency(model.cached_per_million)}</td>
                                        <td>{formatCurrency(model.reasoning_per_million)}</td>
                                    </tr>
                                ))}
                                {models.length === 0 ? (
                                    <tr>
                                        <td colSpan={5}>No pricing models loaded.</td>
                                    </tr>
                                ) : null}
                            </tbody>
                        </table>
                    </div>
                </div>
            </div>
        </div>
    );
}

export default Pricing;
