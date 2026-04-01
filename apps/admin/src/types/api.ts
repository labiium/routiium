// API Key types for routiium authentication

export interface ApiKey {
  id: string;
  name: string;
  prefix: string;
  key: string;
  createdAt: string;
  lastUsed?: string;
  expiresAt?: string;
  rateLimit: number;
  status: ApiKeyStatus;
  requests: number;
  routes: string[];
  metadata?: ApiKeyMetadata;
}

export type ApiKeyStatus = 'active' | 'expired' | 'revoked' | 'suspended';

export interface ApiKeyMetadata {
  description?: string;
  tags?: string[];
  allowedIps?: string[];
  allowedOrigins?: string[];
  rateLimitWindow?: number; // in seconds
  quota?: number; // total requests allowed
  quotaUsed?: number;
}

export interface CreateApiKeyRequest {
  name: string;
  rateLimit?: number;
  expiresAt?: string;
  routes?: string[];
  metadata?: ApiKeyMetadata;
}

export interface GeneratedKey {
  key: string;
  prefix: string;
  id: string;
}

export interface ApiKeyUsage {
  apiKeyId: string;
  totalRequests: number;
  totalTokens: number;
  totalCost: number;
  period: string;
  dailyUsage: DailyUsage[];
}

export interface DailyUsage {
  date: string;
  requests: number;
  tokens: number;
  cost: number;
}

export interface ApiKeyVerification {
  valid: boolean;
  apiKey?: ApiKey;
  reason?: string;
}

export interface ApiKeyStats {
  totalKeys: number;
  activeKeys: number;
  expiredKeys: number;
  revokedKeys: number;
  totalRequests: number;
  totalCost: number;
}

export interface ApiKeyFilter {
  status?: ApiKeyStatus;
  search?: string;
  createdAfter?: string;
  createdBefore?: string;
  expiresBefore?: string;
  expiresAfter?: string;
}
