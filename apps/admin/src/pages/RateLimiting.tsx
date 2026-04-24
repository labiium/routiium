import { useMemo, useState } from "react";
import JsonEditorCard from "../components/JsonEditorCard";
import { ErrorState, LoadingState } from "../components/AsyncState";
import { sendJson, fetchJson } from "../lib/adminApi";
import { useAdminPanelState } from "../hooks/useAdminPanelState";

const EMPTY_POLICY = {
    id: "new-policy",
    buckets: [
        {
            name: "minute",
            requests: 60,
            window_seconds: 60,
            window_type: "Fixed",
        },
    ],
};

function RateLimiting() {
    const { data, error, isLoading, refresh } = useAdminPanelState();
    const [selectedPolicyId, setSelectedPolicyId] = useState<string>("new-policy");
    const [assignmentKey, setAssignmentKey] = useState("");
    const [assignmentPolicy, setAssignmentPolicy] = useState("");
    const [blockKey, setBlockKey] = useState("");
    const [blockDuration, setBlockDuration] = useState("3600");
    const [blockReason, setBlockReason] = useState("Emergency block by admin");

    const policies = useMemo(() => data?.rate_limits?.policies || [], [data?.rate_limits?.policies]);
    const selectedPolicy = useMemo(() => {
        return policies.find((policy: any) => policy.id === selectedPolicyId) || EMPTY_POLICY;
    }, [policies, selectedPolicyId]);

    if (isLoading) {
        return (
            <LoadingState
                title="Rate Limiting"
                description="Manage Routiium rate-limit policies and emergency blocks."
            />
        );
    }

    if (error || !data) {
        return (
            <ErrorState
                title="Rate Limiting"
                description="Manage Routiium rate-limit policies and emergency blocks."
                error={error}
                onRetry={refresh}
            />
        );
    }

    const emergencyBlocks = data.rate_limits?.emergency_blocks || [];
    const defaultPolicyId = data.rate_limits?.default_policy_id;

    return (
        <div>
            <div className="page-header">
                <div className="page-header-row">
                    <div>
                        <h1 className="page-title">Rate Limiting</h1>
                        <p className="page-description">
                            Live policy editing, default assignment, key assignment, and emergency
                            blocking.
                        </p>
                    </div>
                    <div className="page-actions">
                        <button
                            className="btn btn-secondary"
                            onClick={async () => {
                                await sendJson("/admin/rate-limits/reload", "POST");
                                await refresh();
                            }}
                        >
                            Reload File Config
                        </button>
                    </div>
                </div>
            </div>

            <div className="card" style={{ marginBottom: "var(--space-6)" }}>
                <div className="card-header">
                    <div>
                        <h3 className="card-title">Policy Selection</h3>
                        <p className="card-description">
                            Default policy: {defaultPolicyId || "none"}
                        </p>
                    </div>
                    <div style={{ display: "flex", gap: "var(--space-3)" }}>
                        <select
                            className="form-select"
                            value={selectedPolicyId}
                            onChange={(event) => setSelectedPolicyId(event.target.value)}
                        >
                            <option value="new-policy">New Policy</option>
                            {policies.map((policy: any) => (
                                <option key={policy.id} value={policy.id}>
                                    {policy.id}
                                </option>
                            ))}
                        </select>
                        {selectedPolicyId !== "new-policy" ? (
                            <>
                                <button
                                    className="btn btn-secondary"
                                    onClick={async () => {
                                        await sendJson("/admin/rate-limits/default", "POST", {
                                            policy_id: selectedPolicyId,
                                        });
                                        await refresh();
                                    }}
                                >
                                    Set Default
                                </button>
                                <button
                                    className="btn btn-ghost"
                                    onClick={async () => {
                                        await sendJson(
                                            `/admin/rate-limits/policies/${selectedPolicyId}`,
                                            "DELETE",
                                        );
                                        setSelectedPolicyId("new-policy");
                                        await refresh();
                                    }}
                                >
                                    Delete
                                </button>
                            </>
                        ) : null}
                    </div>
                </div>
            </div>

            <JsonEditorCard
                title="Policy JSON"
                description="Policies map directly to Routiium’s `RateLimitPolicy` schema."
                value={selectedPolicy}
                onSave={async (value: any) => {
                    if (!value?.id) {
                        throw new Error("Policy JSON must include an id field.");
                    }

                    const exists = policies.some((policy: any) => policy.id === value.id);
                    if (exists) {
                        await sendJson(`/admin/rate-limits/policies/${value.id}`, "PUT", value);
                    } else {
                        await sendJson("/admin/rate-limits/policies", "POST", value);
                    }
                    setSelectedPolicyId(value.id);
                    await refresh();
                }}
            />

            <div className="section" style={{ display: "grid", gap: "var(--space-6)" }}>
                <div className="card">
                    <div className="card-header">
                        <div>
                            <h3 className="card-title">Assign Policy To Key</h3>
                            <p className="card-description">
                                Bind or remove a policy assignment for a specific API key.
                            </p>
                        </div>
                    </div>
                    <div className="card-content">
                        <div className="two-column">
                            <div className="form-group">
                                <label className="form-label">Key ID</label>
                                <input
                                    className="form-input"
                                    value={assignmentKey}
                                    onChange={(event) => setAssignmentKey(event.target.value)}
                                />
                            </div>
                            <div className="form-group">
                                <label className="form-label">Policy ID</label>
                                <select
                                    className="form-select"
                                    value={assignmentPolicy}
                                    onChange={(event) => setAssignmentPolicy(event.target.value)}
                                >
                                    <option value="">Select policy</option>
                                    {policies.map((policy: any) => (
                                        <option key={policy.id} value={policy.id}>
                                            {policy.id}
                                        </option>
                                    ))}
                                </select>
                            </div>
                        </div>
                        <div style={{ display: "flex", gap: "var(--space-3)" }}>
                            <button
                                className="btn btn-primary"
                                onClick={async () => {
                                    await sendJson(
                                        `/admin/rate-limits/keys/${assignmentKey}`,
                                        "POST",
                                        { policy_id: assignmentPolicy },
                                    );
                                    if (assignmentKey) {
                                        await fetchJson(
                                            `/admin/rate-limits/keys/${assignmentKey}/status`,
                                        );
                                    }
                                    await refresh();
                                }}
                                disabled={!assignmentKey || !assignmentPolicy}
                            >
                                Assign
                            </button>
                            <button
                                className="btn btn-secondary"
                                onClick={async () => {
                                    await sendJson(
                                        `/admin/rate-limits/keys/${assignmentKey}`,
                                        "DELETE",
                                    );
                                    await refresh();
                                }}
                                disabled={!assignmentKey}
                            >
                                Remove Assignment
                            </button>
                        </div>
                    </div>
                </div>

                <div className="card">
                    <div className="card-header">
                        <div>
                            <h3 className="card-title">Emergency Block</h3>
                            <p className="card-description">
                                Immediately block a key for a fixed duration.
                            </p>
                        </div>
                    </div>
                    <div className="card-content">
                        <div className="two-column">
                            <div className="form-group">
                                <label className="form-label">Key ID</label>
                                <input
                                    className="form-input"
                                    value={blockKey}
                                    onChange={(event) => setBlockKey(event.target.value)}
                                />
                            </div>
                            <div className="form-group">
                                <label className="form-label">Duration (seconds)</label>
                                <input
                                    className="form-input"
                                    value={blockDuration}
                                    onChange={(event) => setBlockDuration(event.target.value)}
                                />
                            </div>
                            <div className="form-group" style={{ gridColumn: "1 / -1" }}>
                                <label className="form-label">Reason</label>
                                <input
                                    className="form-input"
                                    value={blockReason}
                                    onChange={(event) => setBlockReason(event.target.value)}
                                />
                            </div>
                        </div>
                        <button
                            className="btn btn-primary"
                            onClick={async () => {
                                await sendJson("/admin/rate-limits/emergency", "POST", {
                                    key_id: blockKey,
                                    duration_secs: Number(blockDuration) || 3600,
                                    reason: blockReason,
                                });
                                await refresh();
                            }}
                            disabled={!blockKey}
                        >
                            Block Key
                        </button>
                    </div>
                </div>
            </div>

            <div className="section">
                <div className="card">
                    <div className="card-header">
                        <div>
                            <h3 className="card-title">Emergency Blocks</h3>
                            <p className="card-description">Currently blocked keys.</p>
                        </div>
                    </div>
                    <div className="card-content">
                        <div className="table-container">
                            <table className="table">
                                <thead>
                                    <tr>
                                        <th>Key</th>
                                        <th>Until</th>
                                        <th>Reason</th>
                                        <th></th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {emergencyBlocks.map((block: any) => (
                                        <tr key={block.key_id}>
                                            <td>{block.key_id}</td>
                                            <td>{block.until_secs}</td>
                                            <td>{block.reason}</td>
                                            <td>
                                                <button
                                                    className="btn btn-ghost"
                                                    onClick={async () => {
                                                        await sendJson(
                                                            `/admin/rate-limits/emergency/${block.key_id}`,
                                                            "DELETE",
                                                        );
                                                        await refresh();
                                                    }}
                                                >
                                                    Remove
                                                </button>
                                            </td>
                                        </tr>
                                    ))}
                                    {emergencyBlocks.length === 0 ? (
                                        <tr>
                                            <td colSpan={4}>No active emergency blocks.</td>
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

export default RateLimiting;
