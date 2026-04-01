import { useCallback, useEffect, useState } from "react";
import { ErrorState, LoadingState } from "../components/AsyncState";
import { fetchBlob, fetchJson } from "../lib/adminApi";
import { formatCurrency, formatDateTime, formatNumber } from "../lib/formatters";

function Analytics() {
    const [stats, setStats] = useState<any>(null);
    const [aggregate, setAggregate] = useState<any>(null);
    const [events, setEvents] = useState<any[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [error, setError] = useState<string | null>(null);

    const load = useCallback(async () => {
        const now = Math.floor(Date.now() / 1000);
        const start = now - 24 * 60 * 60;

        try {
            setError(null);
            setIsLoading(true);
            const [statsPayload, aggregatePayload, eventsPayload] = await Promise.all([
                fetchJson("/analytics/stats"),
                fetchJson(`/analytics/aggregate?start=${start}&end=${now}`),
                fetchJson(`/analytics/events?start=${start}&end=${now}&limit=100`),
            ]);
            setStats(statsPayload);
            setAggregate(aggregatePayload);
            setEvents(eventsPayload.events || []);
        } catch (err) {
            setError(err instanceof Error ? err.message : "Failed to load analytics");
        } finally {
            setIsLoading(false);
        }
    }, []);

    useEffect(() => {
        load();
    }, [load]);

    if (isLoading) {
        return (
            <LoadingState
                title="Analytics"
                description="Recent gateway usage, latency, and cost metrics."
            />
        );
    }

    if (error) {
        return (
            <ErrorState
                title="Analytics"
                description="Recent gateway usage, latency, and cost metrics."
                error={error}
                onRetry={load}
            />
        );
    }

    return (
        <div>
            <div className="page-header">
                <div className="page-header-row">
                    <div>
                        <h1 className="page-title">Analytics</h1>
                        <p className="page-description">
                            Rolling 24-hour analytics from Routiium’s configured analytics backend.
                        </p>
                    </div>
                    <div className="page-actions">
                        <button className="btn btn-secondary" onClick={load}>
                            Refresh
                        </button>
                        <button
                            className="btn btn-primary"
                            onClick={async () => {
                                const blob = await fetchBlob("/analytics/export?format=csv");
                                const url = URL.createObjectURL(blob);
                                const link = document.createElement("a");
                                link.href = url;
                                link.download = "routiium-analytics.csv";
                                link.click();
                                URL.revokeObjectURL(url);
                            }}
                        >
                            Export CSV
                        </button>
                    </div>
                </div>
            </div>

            <div className="stats-grid">
                <div className="stat-card">
                    <div className="stat-label">Total Events</div>
                    <div className="stat-value">{formatNumber(stats?.total_events)}</div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Requests (24h)</div>
                    <div className="stat-value">{formatNumber(aggregate?.total_requests)}</div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Average Duration</div>
                    <div className="stat-value">
                        {formatNumber(aggregate?.avg_duration_ms)}ms
                    </div>
                </div>
                <div className="stat-card">
                    <div className="stat-label">Tracked Cost</div>
                    <div className="stat-value">{formatCurrency(aggregate?.total_cost)}</div>
                </div>
            </div>

            <div className="card">
                <div className="card-header">
                    <div>
                        <h3 className="card-title">Recent Events</h3>
                        <p className="card-description">Latest request events captured by analytics.</p>
                    </div>
                </div>
                <div className="card-content">
                    <div className="table-container">
                        <table className="table">
                            <thead>
                                <tr>
                                    <th>Time</th>
                                    <th>Endpoint</th>
                                    <th>Model</th>
                                    <th>Status</th>
                                    <th>Duration</th>
                                    <th>Cost</th>
                                    <th>Key</th>
                                </tr>
                            </thead>
                            <tbody>
                                {events.map((event) => (
                                    <tr key={event.id}>
                                        <td>{formatDateTime(event.timestamp)}</td>
                                        <td>{event.request?.endpoint}</td>
                                        <td>{event.request?.model || "—"}</td>
                                        <td>{event.response?.status_code}</td>
                                        <td>{formatNumber(event.performance?.duration_ms)}ms</td>
                                        <td>{formatCurrency(event.cost?.total_cost)}</td>
                                        <td>{event.auth?.api_key_label || event.auth?.api_key_id || "—"}</td>
                                    </tr>
                                ))}
                                {events.length === 0 ? (
                                    <tr>
                                        <td colSpan={7}>No events recorded for this window.</td>
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

export default Analytics;
