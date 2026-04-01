const API_BASE_URL_KEY = "routiium.admin.apiBaseUrl";
const ADMIN_TOKEN_KEY = "routiium.admin.token";

function browserDefaultBaseUrl() {
    if (typeof window === "undefined") {
        return "http://localhost:8088";
    }

    return window.location.origin;
}

export function getApiBaseUrl() {
    if (typeof window === "undefined") {
        return browserDefaultBaseUrl();
    }

    const stored = window.localStorage.getItem(API_BASE_URL_KEY)?.trim();
    return stored || browserDefaultBaseUrl();
}

export function setApiBaseUrl(value: string) {
    if (typeof window === "undefined") {
        return;
    }

    const normalized = value.trim();
    if (normalized) {
        window.localStorage.setItem(API_BASE_URL_KEY, normalized);
    } else {
        window.localStorage.removeItem(API_BASE_URL_KEY);
    }
}

export function getAdminToken() {
    if (typeof window === "undefined") {
        return "";
    }

    return window.localStorage.getItem(ADMIN_TOKEN_KEY)?.trim() || "";
}

export function setAdminToken(value: string) {
    if (typeof window === "undefined") {
        return;
    }

    const normalized = value.trim();
    if (normalized) {
        window.localStorage.setItem(ADMIN_TOKEN_KEY, normalized);
    } else {
        window.localStorage.removeItem(ADMIN_TOKEN_KEY);
    }
}

function buildUrl(path: string) {
    if (path.startsWith("http://") || path.startsWith("https://")) {
        return path;
    }

    return `${getApiBaseUrl()}${path}`;
}

async function parseError(response: Response) {
    const text = await response.text();

    try {
        const payload = JSON.parse(text);
        if (payload?.error?.message) {
            return payload.error.message as string;
        }
        if (payload?.message) {
            return payload.message as string;
        }
    } catch {
        // Ignore JSON parse failures and fall back to raw text.
    }

    return text || `Request failed with status ${response.status}`;
}

async function request<T>(path: string, options: RequestInit = {}) {
    const headers = new Headers(options.headers);
    const token = getAdminToken();
    if (token && !headers.has("Authorization")) {
        headers.set("Authorization", `Bearer ${token}`);
    }

    const hasBody = options.body !== undefined && options.body !== null;
    if (hasBody && !(options.body instanceof FormData) && !headers.has("Content-Type")) {
        headers.set("Content-Type", "application/json");
    }

    const response = await fetch(buildUrl(path), {
        ...options,
        headers,
    });

    if (!response.ok) {
        throw new Error(await parseError(response));
    }

    if (response.status === 204) {
        return undefined as T;
    }

    const contentType = response.headers.get("content-type") || "";
    if (contentType.includes("application/json")) {
        return (await response.json()) as T;
    }

    return undefined as T;
}

export function fetchJson<T>(path: string) {
    return request<T>(path);
}

export function sendJson<T>(path: string, method: string, body?: unknown) {
    return request<T>(path, {
        method,
        body: body === undefined ? undefined : JSON.stringify(body),
    });
}

export async function fetchBlob(path: string) {
    const headers = new Headers();
    const token = getAdminToken();
    if (token) {
        headers.set("Authorization", `Bearer ${token}`);
    }

    const response = await fetch(buildUrl(path), { headers });
    if (!response.ok) {
        throw new Error(await parseError(response));
    }

    return response.blob();
}

export function broadcastAdminConfigChanged() {
    if (typeof window === "undefined") {
        return;
    }

    window.dispatchEvent(new Event("routiium-admin-config-changed"));
}
