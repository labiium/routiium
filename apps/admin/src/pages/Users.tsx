import { ErrorState, LoadingState } from "../components/AsyncState";
import { formatDateTime, formatNumber } from "../lib/formatters";
import { useAdminPanelState } from "../hooks/useAdminPanelState";

function Users() {
    const { data, error, isLoading, refresh } = useAdminPanelState();

    if (isLoading) {
        return (
            <LoadingState
                title="Principals"
                description="Derived API-key principals and recent authenticated activity."
            />
        );
    }

    if (error || !data) {
        return (
            <ErrorState
                title="Principals"
                description="Derived API-key principals and recent authenticated activity."
                error={error}
                onRetry={refresh}
            />
        );
    }

    const principals = data.principals?.items || [];

    return (
        <div>
            <div className="page-header">
                <div className="page-header-row">
                    <div>
                        <h1 className="page-title">Principals</h1>
                        <p className="page-description">{data.principals?.note}</p>
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
                    <div className="stat-label">Observed Principals</div>
                    <div className="stat-value">{formatNumber(principals.length)}</div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Sample Window</div>
                    <div className="stat-value">
                        {formatDateTime(data.principals?.sample_window_start)}
                    </div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Window End</div>
                    <div className="stat-value">
                        {formatDateTime(data.principals?.sample_window_end)}
                    </div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Sample Size</div>
                    <div className="stat-value">
                        {formatNumber(data.principals?.sample_limit)}
                    </div>
                </div>
            </div>

            <div className="card">
                <div className="card-content">
                    <div className="table-container">
                        <table className="table">
                            <thead>
                                <tr>
                                    <th>Principal</th>
                                    <th>Status</th>
                                    <th>Rate Limit Policy</th>
                                    <th>Requests (30d sample)</th>
                                    <th>Last Seen</th>
                                    <th>Models</th>
                                </tr>
                            </thead>
                            <tbody>
                                {principals.map((principal: any) => (
                                    <tr key={principal.id}>
                                        <td>{principal.label || principal.id}</td>
                                        <td>{principal.status}</td>
                                        <td>{principal.rate_limit_policy || "—"}</td>
                                        <td>{formatNumber(principal.request_count_30d)}</td>
                                        <td>{formatDateTime(principal.last_seen_at)}</td>
                                        <td>{(principal.models_used || []).join(", ") || "—"}</td>
                                    </tr>
                                ))}
                                {principals.length === 0 ? (
                                    <tr>
                                        <td colSpan={6}>No principals observed.</td>
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

export default Users;
