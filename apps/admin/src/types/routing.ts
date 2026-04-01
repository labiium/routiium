// Routing configuration types for routiium

export interface RoutingConfig {
  id: string;
  name: string;
  version: string;
  rules: RoutingRule[];
  defaultRoute?: string;
  loadBalanceStrategy?: LoadBalanceStrategy;
  createdAt: string;
  updatedAt: string;
}

export interface RoutingRule {
  id: string;
  name: string;
  path: string;
  pathPattern?: string;
  method?: HttpMethod;
  target: string;
  priority: number;
  active: boolean;
  auth?: AuthType;
  rateLimit?: number;
  description?: string;
  transforms?: RequestTransform[];
  headers?: Record<string, string>;
  matchStrategy?: MatchStrategy;
  conditions?: RoutingCondition[];
  fallback?: FallbackConfig;
}

export type HttpMethod = 'GET' | 'POST' | 'PUT' | 'DELETE' | 'PATCH' | 'OPTIONS' | 'HEAD' | '*';

export type AuthType = 'none' | 'key_required' | 'key_optional' | 'jwt' | 'oauth';

export type LoadBalanceStrategy = 'round_robin' | 'random' | 'least_connections' | 'weighted' | 'ip_hash';

export type MatchStrategy = 'exact' | 'prefix' | 'regex' | 'wildcard';

export interface RequestTransform {
  type: 'header' | 'body' | 'query' | 'path';
  operation: 'set' | 'remove' | 'replace' | 'rename';
  key: string;
  value?: string;
  pattern?: string;
  replacement?: string;
}

export interface RoutingCondition {
  type: 'header' | 'query' | 'body' | 'path' | 'ip';
  key: string;
  operator: 'equals' | 'contains' | 'regex' | 'exists' | 'not_exists';
  value: string;
}

export interface FallbackConfig {
  enabled: boolean;
  targets: string[];
  retryCount?: number;
  timeout?: number;
}

export interface ResolvedRoute {
  rule: RoutingRule;
  target: string;
  transforms: RequestTransform[];
  metadata?: RouteMetadata;
}

export interface RouteMetadata {
  latency?: number;
  load?: number;
  status?: 'healthy' | 'degraded' | 'unhealthy';
  lastChecked?: string;
}

export interface RouteStats {
  routeId: string;
  totalRequests: number;
  successfulRequests: number;
  failedRequests: number;
  averageLatency: number;
  p50Latency: number;
  p95Latency: number;
  p99Latency: number;
  requestsPerMinute: number;
  errorRate: number;
}

export interface BackendConfig {
  url: string;
  weight?: number;
  maxConnections?: number;
  timeout?: number;
  healthCheck?: HealthCheckConfig;
  auth?: BackendAuth;
}

export interface HealthCheckConfig {
  enabled: boolean;
  interval: number; // seconds
  timeout: number;
  healthyThreshold: number;
  unhealthyThreshold: number;
  endpoint?: string;
}

export interface BackendAuth {
  type: 'none' | 'api_key' | 'basic' | 'bearer';
  credentials?: Record<string, string>;
}

export interface ModelAlias {
  alias: string;
  models: string[];
  description?: string;
  defaultModel?: string;
}

export interface RouterCache {
  enabled: boolean;
  ttl: number;
  maxSize: number;
  strategy?: 'lru' | 'lfu' | 'fifo';
}

export interface UpstreamConfig {
  baseUrl: string;
  timeout?: number;
  retries?: number;
  headers?: Record<string, string>;
}

export interface RouteRequest {
  path: string;
  method: HttpMethod;
  headers?: Record<string, string>;
  body?: unknown;
  query?: Record<string, string>;
  sourceIp?: string;
}

export interface RoutePlan {
  routes: ResolvedRoute[];
  selectedRoute?: ResolvedRoute;
  reasoning?: string;
}

export interface RouteHints {
  preferredModels?: string[];
  preferredProviders?: string[];
  maxLatency?: number;
  maxCost?: number;
}

export interface RouteLimits {
  maxTokens?: number;
  maxRequestsPerMinute?: number;
  maxConcurrentRequests?: number;
}

export interface RouteFeedback {
  routeId: string;
  success: boolean;
  latency?: number;
  error?: string;
  metadata?: Record<string, unknown>;
}
