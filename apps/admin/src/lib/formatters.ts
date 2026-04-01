export function formatDateTime(value?: string | number | null) {
    if (value === null || value === undefined || value === "") {
        return "—";
    }

    const date = typeof value === "number" ? new Date(value * 1000) : new Date(value);
    if (Number.isNaN(date.getTime())) {
        return String(value);
    }

    return date.toLocaleString();
}

export function formatNumber(value?: number | null) {
    if (value === null || value === undefined || Number.isNaN(value)) {
        return "0";
    }

    return new Intl.NumberFormat().format(value);
}

export function formatCurrency(value?: number | null, currency = "USD") {
    if (value === null || value === undefined || Number.isNaN(value)) {
        return "$0.00";
    }

    return new Intl.NumberFormat(undefined, {
        style: "currency",
        currency,
        maximumFractionDigits: 4,
    }).format(value);
}

export function formatJson(value: unknown) {
    return JSON.stringify(value ?? null, null, 2) ?? "null";
}

export function truncateMiddle(value: string, keep = 8) {
    if (value.length <= keep * 2) {
        return value;
    }

    return `${value.slice(0, keep)}…${value.slice(-keep)}`;
}
