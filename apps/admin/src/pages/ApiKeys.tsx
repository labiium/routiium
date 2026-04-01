import { useCallback, useEffect, useMemo, useState } from "react";
import { ErrorState, LoadingState } from "../components/AsyncState";
import { fetchJson, sendJson } from "../lib/adminApi";
import { formatDateTime, truncateMiddle } from "../lib/formatters";

function ApiKeys() {
    const [keys, setKeys] = useState<any[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [error, setError] = useState<string | null>(null);
    const [label, setLabel] = useState("");
    const [ttlHours, setTtlHours] = useState("24");
    const [scopes, setScopes] = useState("");
    const [generated, setGenerated] = useState<any | null>(null);

    const loadKeys = useCallback(async () => {
        try {
            setError(null);
            setIsLoading(true);
            const payload = await fetchJson<any[]>("/keys");
            setKeys(payload);
        } catch (err) {
            setError(err instanceof Error ? err.message : "Failed to load API keys");
        } finally {
            setIsLoading(false);
        }
    }, []);

    useEffect(() => {
        loadKeys();
    }, [loadKeys]);

    const rows = useMemo(() => {
        const now = Math.floor(Date.now() / 1000);
        return keys.map((key) => {
            let status = "active";
            if (key.revoked_at) {
                status = "revoked";
            } else if (key.expires_at && key.expires_at <= now) {
                status = "expired";
            }

            return { ...key, status };
        });
    }, [keys]);

    if (isLoading) {
        return (
            <LoadingState
                title="API Keys"
                description="Generate, inspect, and revoke Routiium API keys."
            />
        );
    }

    if (error) {
        return (
            <ErrorState
                title="API Keys"
                description="Generate, inspect, and revoke Routiium API keys."
                error={error}
                onRetry={loadKeys}
            />
        );
    }

    return (
        <div>
            <div className="page-header">
                <div className="page-header-row">
                    <div>
                        <h1 className="page-title">API Keys</h1>
                        <p className="page-description">
                            Live API key inventory from Routiium’s key store.
                        </p>
                    </div>
                    <div className="page-actions">
                        <button className="btn btn-secondary" onClick={loadKeys}>
                            Refresh
                        </button>
                    </div>
                </div>
            </div>

            <div className="card" style={{ marginBottom: "var(--space-6)" }}>
                <div className="card-header">
                    <div>
                        <h3 className="card-title">Generate Key</h3>
                        <p className="card-description">
                            Create a managed Routiium key. TTL is optional; scopes are comma-separated.
                        </p>
                    </div>
                </div>
                <div className="card-content">
                    <div className="two-column">
                        <div className="form-group">
                            <label className="form-label">Label</label>
                            <input
                                className="form-input"
                                value={label}
                                onChange={(event) => setLabel(event.target.value)}
                                placeholder="production-app"
                            />
                        </div>
                        <div className="form-group">
                            <label className="form-label">TTL Hours</label>
                            <input
                                className="form-input"
                                value={ttlHours}
                                onChange={(event) => setTtlHours(event.target.value)}
                                placeholder="24"
                            />
                        </div>
                        <div className="form-group" style={{ gridColumn: "1 / -1" }}>
                            <label className="form-label">Scopes</label>
                            <input
                                className="form-input"
                                value={scopes}
                                onChange={(event) => setScopes(event.target.value)}
                                placeholder="read,write"
                            />
                        </div>
                    </div>
                    <div style={{ display: "flex", gap: "var(--space-3)" }}>
                        <button
                            className="btn btn-primary"
                            onClick={async () => {
                                const ttl = Number(ttlHours);
                                const payload = await sendJson<any>("/keys/generate", "POST", {
                                    label: label || null,
                                    ttl_seconds: Number.isFinite(ttl) && ttl > 0 ? ttl * 3600 : undefined,
                                    scopes: scopes
                                        .split(",")
                                        .map((value) => value.trim())
                                        .filter(Boolean),
                                });
                                setGenerated(payload);
                                setLabel("");
                                setScopes("");
                                await loadKeys();
                            }}
                        >
                            Generate
                        </button>
                    </div>
                    {generated ? (
                        <div className="alert alert-success" style={{ marginTop: "var(--space-4)" }}>
                            <div style={{ display: "grid", gap: "var(--space-2)" }}>
                                <strong>Generated key</strong>
                                <code>{generated.token}</code>
                                <div>
                                    <button
                                        className="btn btn-secondary"
                                        onClick={() => navigator.clipboard.writeText(generated.token)}
                                    >
                                        Copy Token
                                    </button>
                                </div>
                            </div>
                        </div>
                    ) : null}
                </div>
            </div>

            <div className="card">
                <div className="card-content">
                    <div className="table-container">
                        <table className="table">
                            <thead>
                                <tr>
                                    <th>Label</th>
                                    <th>ID</th>
                                    <th>Status</th>
                                    <th>Policy</th>
                                    <th>Scopes</th>
                                    <th>Created</th>
                                    <th>Expires</th>
                                    <th></th>
                                </tr>
                            </thead>
                            <tbody>
                                {rows.map((key) => (
                                    <tr key={key.id}>
                                        <td>{key.label || "—"}</td>
                                        <td>{truncateMiddle(key.id)}</td>
                                        <td>{key.status}</td>
                                        <td>{key.rate_limit_policy || "—"}</td>
                                        <td>{(key.scopes || []).join(", ") || "—"}</td>
                                        <td>{formatDateTime(key.created_at)}</td>
                                        <td>{formatDateTime(key.expires_at)}</td>
                                        <td>
                                            <button
                                                className="btn btn-ghost"
                                                disabled={key.status === "revoked"}
                                                onClick={async () => {
                                                    await sendJson("/keys/revoke", "POST", { id: key.id });
                                                    await loadKeys();
                                                }}
                                            >
                                                Revoke
                                            </button>
                                        </td>
                                    </tr>
                                ))}
                                {rows.length === 0 ? (
                                    <tr>
                                        <td colSpan={8}>No keys found.</td>
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

export default ApiKeys;
