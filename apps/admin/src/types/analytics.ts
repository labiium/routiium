// Analytics types for Routiium API

// Core Analytics Event from Routiium
export interface AnalyticsEvent {
    id: string;
    timestamp: number; // Unix timestamp in seconds
    request: RequestMetadata;
    response: ResponseMetadata;
    performance: PerformanceMetrics;
    token_usage?: TokenUsage;
    cost?: CostInfo;
    auth?: AuthMetadata;
    routing?: RoutingMetadata;
}

export interface RequestMetadata {
    endpoint: string;
    method: string;
    model: string;
    stream: boolean;
    size_bytes: number;
    message_count?: number;
    input_tokens?: number;
    user_agent?: string;
    client_ip?: string;
}

export interface ResponseMetadata {
    status_code: number;
    size_bytes: number;
    output_tokens?: number;
    success: boolean;
    error_message?: string | null;
}

export interface PerformanceMetrics {
    duration_ms: number;
    ttfb_ms?: number | null;
    upstream_duration_ms?: number;
    tokens_per_second?: number;
}

export interface TokenUsage {
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
    cached_tokens?: number;
    reasoning_tokens?: number | null;
}

export interface CostInfo {
    input_cost: number;
    output_cost: number;
    cached_cost: number;
    total_cost: number;
    currency: string;
    pricing_model?: string;
}

export interface AuthMetadata {
    authenticated: boolean;
    api_key_id?: string;
    api_key_label?: string;
    auth_method?: string;
}

export interface RoutingMetadata {
    backend: string;
    upstream_mode?: string;
    mcp_enabled?: boolean;
    mcp_servers?: string[];
    system_prompt_applied?: boolean;
}

// API Response Types

export interface AnalyticsStats {
    total_events: number;
    backend_type: string;
    ttl_seconds?: number;
    max_events?: number | null;
    total_cost: number;
    total_input_tokens: number;
    total_output_tokens: number;
    total_cached_tokens: number;
    total_reasoning_tokens: number;
    avg_tokens_per_second: number;
}

export interface AnalyticsAggregate {
    total_requests: number;
    successful_requests: number;
    failed_requests: number;
    total_input_tokens: number;
    total_output_tokens: number;
    total_cached_tokens: number;
    total_reasoning_tokens: number;
    avg_duration_ms: number;
    avg_tokens_per_second: number;
    total_cost: number;
    cost_by_model: Record<string, number>;
    models_used: Record<string, number>;
    endpoints_hit: Record<string, number>;
    backends_used: Record<string, number>;
    period_start: number;
    period_end: number;
}

export interface AnalyticsEventsResponse {
    events: AnalyticsEvent[];
    count: number;
    start: number;
    end: number;
}

// Query Parameters

export interface TimeRangeParams {
    start?: number; // Unix timestamp in seconds
    end?: number; // Unix timestamp in seconds
    limit?: number;
}

export interface AnalyticsExportParams {
    start?: number;
    end?: number;
    format?: "json" | "csv";
}

export interface AnalyticsFilter {
    startDate?: string;
    endDate?: string;
    apiKeyId?: string;
    userId?: string;
    routeId?: string;
    model?: string;
    provider?: string;
    eventType?: string;
    minLatency?: number;
    maxLatency?: number;
    status?: "success" | "error";
}

// Time Series Data

export interface TimeSeriesDataPoint {
    timestamp: string;
    value: number;
    label?: string;
}

export interface AnalyticsTimeSeries {
    metric: string;
    data: TimeSeriesDataPoint[];
    interval: "minute" | "hour" | "day" | "week" | "month";
}

// Top Entities

export interface TopModels {
    model: string;
    requests: number;
    tokens: number;
    cost: number;
    percentage: number;
}

export interface TopRoutes {
    routeId: string;
    routeName: string;
    requests: number;
    percentage: number;
    averageLatency: number;
}

export interface TopUsers {
    userId: string;
    requests: number;
    tokens: number;
    cost: number;
    percentage: number;
}

// Provider Metrics

export interface ProviderMetrics {
    provider: string;
    requests: number;
    tokens: number;
    cost: number;
    averageLatency: number;
    errorRate: number;
    successRate: number;
}

// Latency Distribution

export interface LatencyDistribution {
    p50: number;
    p75: number;
    p90: number;
    p95: number;
    p99: number;
    p999: number;
}

// Dashboard Data

export interface AnalyticsDashboard {
    summary: AnalyticsStats;
    requestTimeSeries: AnalyticsTimeSeries[];
    costTimeSeries: AnalyticsTimeSeries[];
    latencyTimeSeries: AnalyticsTimeSeries[];
    topModels: TopModels[];
    topRoutes: TopRoutes[];
    topUsers: TopUsers[];
    providerMetrics: ProviderMetrics[];
    latencyDistribution: LatencyDistribution;
}

// Export

export interface AnalyticsExportRequest {
    format: "csv" | "json" | "parquet";
    filter?: AnalyticsFilter;
    fields?: string[];
    compression?: "none" | "gzip" | "zip";
}

export interface AnalyticsExport {
    id: string;
    status: "pending" | "processing" | "completed" | "failed";
    format: string;
    fileUrl?: string;
    createdAt: string;
    completedAt?: string;
    error?: string;
}

// Event Types

export type AnalyticsEventType =
    | "request"
    | "response"
    | "error"
    | "token_usage"
    | "cost"
    | "latency"
    | "rate_limit"
    | "auth_failure";

// Frontend Time Series Data (transformed from API)

export interface TimeSeriesRow {
    timestamp: string;
    label: string;
    requests: number;
    latency: number;
    cost: number;
    tokens: number;
    errors: number;
    errorRate: number;
}
