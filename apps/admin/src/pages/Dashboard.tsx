import { Activity, Database, Gauge, Key } from "lucide-react";
import { ErrorState, LoadingState } from "../components/AsyncState";
import { formatNumber, formatDateTime } from "../lib/formatters";
import { useAdminPanelState } from "../hooks/useAdminPanelState";

function Dashboard() {
    const { data, error, isLoading, refresh } = useAdminPanelState();

    if (isLoading) {
        return (
            <LoadingState
                title="Dashboard"
                description="Overview of the Routiium control plane."
            />
        );
    }

    if (error || !data) {
        return (
            <ErrorState
                title="Dashboard"
                description="Overview of the Routiium control plane."
                error={error}
                onRetry={refresh}
            />
        );
    }

    const overview = data.overview;
    const principals = data.principals?.items || [];
    const analyticsStats = overview.analytics?.stats || {};
    const chatHistoryStats = overview.chat_history?.stats || {};
    const routingStats = data.routing?.stats || {};

    const stats = [
        {
            label: "API Keys",
            value: formatNumber(overview.api_keys?.total),
            detail: `${formatNumber(overview.api_keys?.active)} active`,
            icon: Key,
        },
        {
            label: "Rate Limit Policies",
            value: formatNumber(data.rate_limits?.policies?.length),
            detail: `${formatNumber(overview.rate_limits?.emergency_blocks)} emergency blocks`,
            icon: Gauge,
        },
        {
            label: "Analytics Events",
            value: formatNumber(analyticsStats.total_events),
            detail: `${formatNumber(analyticsStats.total_input_tokens)} input tokens`,
            icon: Activity,
        },
        {
            label: "Conversations",
            value: formatNumber(chatHistoryStats.total_conversations),
            detail: `${formatNumber(chatHistoryStats.total_messages)} messages`,
            icon: Database,
        },
    ];

    return (
        <div>
            <div className="page-header">
                <div className="page-header-row">
                    <div>
                        <h1 className="page-title">Dashboard</h1>
                        <p className="page-description">
                            Routiium runtime overview generated at{" "}
                            {formatDateTime(overview.generated_at)}.
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
                {stats.map((stat) => (
                    <div key={stat.label} className="stat-card">
                        <div className="stat-label">
                            <stat.icon />
                            {stat.label}
                        </div>
                        <div className="stat-value">{stat.value}</div>
                        <div className="stat-change positive">{stat.detail}</div>
                    </div>
                ))}
            </div>

            <div className="section">
                <div className="card">
                    <div className="card-header">
                        <div>
                            <h3 className="card-title">Runtime</h3>
                            <p className="card-description">
                                Core Routiium feature availability and routing posture.
                            </p>
                        </div>
                    </div>
                    <div className="card-content">
                        <div className="table-container">
                            <table className="table">
                                <tbody>
                                    <tr>
                                        <td>Bind Address</td>
                                        <td>{overview.bind_addr}</td>
                                    </tr>
                                    <tr>
                                        <td>Router Mode</td>
                                        <td>{data.routing?.router?.mode || "none"}</td>
                                    </tr>
                                    <tr>
                                        <td>Routing Rules</td>
                                        <td>{formatNumber(routingStats.total_rules)}</td>
                                    </tr>
                                    <tr>
                                        <td>Routing Aliases</td>
                                        <td>{formatNumber(routingStats.total_aliases)}</td>
                                    </tr>
                                    <tr>
                                        <td>Admin Token</td>
                                        <td>
                                            {overview.admin_token_configured
                                                ? "Configured"
                                                : "Open admin routes"}
                                        </td>
                                    </tr>
                                    <tr>
                                        <td>MCP</td>
                                        <td>{data.mcp?.enabled ? "Enabled" : "Disabled"}</td>
                                    </tr>
                                    <tr>
                                        <td>System Prompts</td>
                                        <td>
                                            {data.system_prompt?.summary?.enabled
                                                ? "Enabled"
                                                : "Disabled"}
                                        </td>
                                    </tr>
                                    <tr>
                                        <td>Bedrock</td>
                                        <td>{data.bedrock?.enabled ? "Detected" : "Not detected"}</td>
                                    </tr>
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
                            <h3 className="card-title">Top Principals</h3>
                            <p className="card-description">
                                API-key principals observed in recent authenticated traffic.
                            </p>
                        </div>
                    </div>
                    <div className="card-content">
                        <div className="table-container">
                            <table className="table">
                                <thead>
                                    <tr>
                                        <th>Principal</th>
                                        <th>Status</th>
                                        <th>Requests (30d sample)</th>
                                        <th>Last Seen</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {principals.slice(0, 8).map((principal: any) => (
                                        <tr key={principal.id}>
                                            <td>{principal.label || principal.id}</td>
                                            <td>{principal.status}</td>
                                            <td>{formatNumber(principal.request_count_30d)}</td>
                                            <td>{formatDateTime(principal.last_seen_at)}</td>
                                        </tr>
                                    ))}
                                    {principals.length === 0 ? (
                                        <tr>
                                            <td colSpan={4}>No principals observed yet.</td>
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

export default Dashboard;
