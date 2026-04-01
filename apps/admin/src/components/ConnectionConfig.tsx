import { useState } from "react";
import {
    broadcastAdminConfigChanged,
    getAdminToken,
    getApiBaseUrl,
    setAdminToken,
    setApiBaseUrl,
} from "../lib/adminApi";

function ConnectionConfig() {
    const [apiBaseUrl, setApiBaseUrlState] = useState(getApiBaseUrl());
    const [adminToken, setAdminTokenState] = useState(getAdminToken());
    const [saved, setSaved] = useState(false);

    const handleApply = () => {
        setApiBaseUrl(apiBaseUrl);
        setAdminToken(adminToken);
        setSaved(true);
        broadcastAdminConfigChanged();
        window.setTimeout(() => setSaved(false), 1500);
    };

    return (
        <div
            style={{
                display: "flex",
                gap: "var(--space-3)",
                alignItems: "center",
                flexWrap: "wrap",
                justifyContent: "flex-end",
            }}
        >
            <input
                className="form-input"
                style={{ minWidth: 220 }}
                value={apiBaseUrl}
                onChange={(event) => setApiBaseUrlState(event.target.value)}
                placeholder="API base URL"
            />
            <input
                className="form-input"
                style={{ minWidth: 220 }}
                type="password"
                value={adminToken}
                onChange={(event) => setAdminTokenState(event.target.value)}
                placeholder="Admin bearer token"
            />
            <button className="btn btn-secondary" onClick={handleApply}>
                {saved ? "Applied" : "Apply"}
            </button>
        </div>
    );
}

export default ConnectionConfig;
