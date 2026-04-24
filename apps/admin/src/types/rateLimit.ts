import type { HttpMethod } from "./routing";

// Rate limiting types for routiium

export interface RateLimitConfig {
  id: string;
  name: string;
  enabled: boolean;
  global: boolean;
  defaultLimit: RateLimitRule;
  rules: RateLimitRule[];
  strategy: RateLimitStrategy;
  storage: RateLimitStorage;
  createdAt: string;
  updatedAt: string;
}

export interface RateLimitRule {
  id: string;
  name: string;
  path?: string;
  pathPattern?: string;
  method?: HttpMethod;
  apiKeyId?: string;
  userId?: string;
  ip?: string;
  limit: number;
  window: number; // in seconds
  blockDuration?: number; // in seconds
  precision?: number; // sliding window precision
  conditions?: RateLimitCondition[];
}


export interface RateLimitCondition {
  type: 'header' | 'query' | 'path' | 'ip';
  key: string;
  operator: 'equals' | 'contains' | 'regex' | 'exists';
  value: string;
}

export type RateLimitStrategy = 'fixed_window' | 'sliding_window' | 'sliding_log' | 'token_bucket' | 'leaky_bucket';

export interface RateLimitStorage {
  type: 'memory' | 'redis' | 'postgres' | 'sqlite';
  config: RateLimitStorageConfig;
  connected?: boolean;
}

export interface RateLimitStorageConfig {
  // Redis config
  host?: string;
  port?: number;
  password?: string;
  database?: number;
  keyPrefix?: string;

  // Postgres/SQLite config
  connectionString?: string;

  // General config
  maxConnections?: number;
  timeout?: number;
}

export interface RateLimitRecord {
  id: string;
  identifier: string; // apiKeyId, userId, or IP
  identifierType: 'api_key' | 'user' | 'ip';
  ruleId?: string;
  count: number;
  resetAt: string;
  blocked: boolean;
  blockedUntil?: string;
  lastUpdated: string;
}

export interface RateLimitStats {
  totalRequests: number;
  allowedRequests: number;
  deniedRequests: number;
  blockedRequests: number;
  currentUsage: number;
  topIdentifiers: RateLimitIdentifier[];
  byRoute: RateLimitRouteStats[];
  byTimeWindow: Record<string, number>;
}

export interface RateLimitIdentifier {
  identifier: string;
  type: 'api_key' | 'user' | 'ip';
  requests: number;
  denied: number;
  blocked: boolean;
}

export interface RateLimitRouteStats {
  route: string;
  requests: number;
  limited: number;
  percentage: number;
}

export interface RateLimitEvent {
  id: string;
  timestamp: string;
  identifier: string;
  identifierType: 'api_key' | 'user' | 'ip';
  ruleId?: string;
  path?: string;
  method?: string;
  status: 'allowed' | 'denied' | 'blocked';
  limit?: number;
  remaining?: number;
  retryAfter?: number;
}

export interface RateLimitFilter {
  identifier?: string;
  identifierType?: 'api_key' | 'user' | 'ip';
  ruleId?: string;
  status?: 'allowed' | 'denied' | 'blocked';
  startDate?: string;
  endDate?: string;
  path?: string;
}

export interface RateLimitWhitelist {
  id: string;
  name: string;
  type: 'api_key' | 'user' | 'ip' | 'path';
  value: string;
  reason?: string;
  expiresAt?: string;
  createdAt: string;
}

export interface RateLimitBlacklist {
  id: string;
  name: string;
  type: 'api_key' | 'user' | 'ip';
  value: string;
  reason?: string;
  permanent: boolean;
  expiresAt?: string;
  createdAt: string;
}

export interface RateLimitTestRequest {
  identifier: string;
  identifierType: 'api_key' | 'user' | 'ip';
  path?: string;
  method?: string;
  weight?: number;
}

export interface RateLimitTestResult {
  allowed: boolean;
  limit: number;
  remaining: number;
  resetAt: string;
  retryAfter?: number;
  blocked?: boolean;
  blockedUntil?: string;
}
