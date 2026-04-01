import { useCallback, useEffect, useState } from "react";
import { fetchJson } from "../lib/adminApi";

export interface AdminPanelState {
    overview: any;
    system_prompt: any;
    mcp: any;
    routing: any;
    pricing: any;
    settings: any;
    bedrock: any;
    principals: any;
    rate_limits: any;
}

export function useAdminPanelState() {
    const [data, setData] = useState<AdminPanelState | null>(null);
    const [isLoading, setIsLoading] = useState(true);
    const [isRefreshing, setIsRefreshing] = useState(false);
    const [error, setError] = useState<string | null>(null);

    const load = useCallback(async (refresh = false) => {
        if (refresh) {
            setIsRefreshing(true);
        } else {
            setIsLoading(true);
        }

        try {
            setError(null);
            const payload = await fetchJson<AdminPanelState>("/admin/panel/state");
            setData(payload);
        } catch (err) {
            setError(err instanceof Error ? err.message : "Failed to load admin state");
        } finally {
            setIsLoading(false);
            setIsRefreshing(false);
        }
    }, []);

    useEffect(() => {
        load();
    }, [load]);

    useEffect(() => {
        const onConfigChanged = () => {
            load(true);
        };

        window.addEventListener("routiium-admin-config-changed", onConfigChanged);
        return () => window.removeEventListener("routiium-admin-config-changed", onConfigChanged);
    }, [load]);

    return {
        data,
        isLoading,
        isRefreshing,
        error,
        refresh: () => load(true),
    };
}
