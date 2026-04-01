// Pricing types for routiium model pricing and cost management

export interface ModelPricing {
  model: string;
  provider: string;
  inputPrice: number; // per 1M tokens
  outputPrice: number; // per 1M tokens
  currency: string;
  contextWindow: number;
  maxOutputTokens?: number;
  updatedAt: string;
}

export interface PricingConfig {
  id: string;
  name: string;
  enabled: boolean;
  models: ModelPricing[];
  defaultCurrency: string;
  priceMultiplier: number;
  customPricing?: CustomPricing[];
  createdAt: string;
  updatedAt: string;
}

export interface CustomPricing {
  model: string;
  provider?: string;
  inputPrice?: number;
  outputPrice?: number;
  flatFee?: number;
  feeType?: 'per_request' | 'per_token' | 'flat';
}

export interface PricingTier {
  id: string;
  name: string;
  description: string;
  monthlyPrice: number;
  features: PricingTierFeature[];
  limits: PricingTierLimit;
  popular?: boolean;
}

export interface PricingTierFeature {
  name: string;
  included: boolean;
  limit?: number;
}

export interface PricingTierLimit {
  requestsPerMonth?: number;
  tokensPerMonth?: number;
  apiKeys?: number;
  teamMembers?: number;
  customRoutes?: number;
}

export interface UsageRecord {
  id: string;
  userId: string;
  apiKeyId?: string;
  period: string; // YYYY-MM
  totalRequests: number;
  totalInputTokens: number;
  totalOutputTokens: number;
  totalCost: number;
  breakdown: UsageBreakdown[];
}

export interface UsageBreakdown {
  model: string;
  requests: number;
  inputTokens: number;
  outputTokens: number;
  inputCost: number;
  outputCost: number;
  totalCost: number;
}

export interface CostAlert {
  id: string;
  userId: string;
  type: CostAlertType;
  threshold: number;
  currentSpend: number;
  period: 'daily' | 'weekly' | 'monthly';
  triggered: boolean;
  triggeredAt?: string;
  createdAt: string;
}

export type CostAlertType = 'spend_limit' | 'budget_warning' | 'anomaly_detection';

export interface Invoice {
  id: string;
  userId: string;
  number: string;
  status: InvoiceStatus;
  period: string;
  subtotal: number;
  tax: number;
  total: number;
  currency: string;
  items: InvoiceItem[];
  dueDate: string;
  paidAt?: string;
  createdAt: string;
}

export type InvoiceStatus = 'draft' | 'pending' | 'paid' | 'overdue' | 'cancelled';

export interface InvoiceItem {
  description: string;
  quantity: number;
  unitPrice: number;
  total: number;
  metadata?: Record<string, unknown>;
}

export interface BillingInfo {
  userId: string;
  plan: PricingTier;
  paymentMethod: PaymentMethod;
  billingEmail: string;
  billingAddress?: BillingAddress;
  nextBillingDate: string;
  autoRenew: boolean;
}

export interface PaymentMethod {
  type: 'card' | 'bank_transfer' | 'paypal';
  last4?: string;
  brand?: string;
  expiryMonth?: number;
  expiryYear?: number;
}

export interface BillingAddress {
  line1: string;
  line2?: string;
  city: string;
  state?: string;
  postalCode: string;
  country: string;
}

export interface ProviderPricing {
  provider: string;
  models: ModelPricing[];
  lastUpdated: string;
}

export interface PriceComparison {
  model: string;
  providers: ProviderPrice[];
  cheapest: string;
  fastest?: string;
}

export interface ProviderPrice {
  provider: string;
  inputPrice: number;
  outputPrice: number;
  latency?: number;
  availability: number;
}
