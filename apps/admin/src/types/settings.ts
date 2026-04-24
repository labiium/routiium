import type { ChatHistoryStoreType } from "./chatHistory";

// Settings types for routiium admin panel configuration

export interface Settings {
  general: GeneralSettings;
  server: ServerSettings;
  database: DatabaseSettings;
  security: SecuritySettings;
  notifications: NotificationSettings;
  api: ApiSettings;
  chatHistory: ChatHistorySettings;
  mcp: MCPSettings;
  pricing: PricingSettings;
  bedrock?: BedrockSettings;
}

export interface GeneralSettings {
  appName: string;
  appVersion: string;
  environment: 'development' | 'staging' | 'production';
  timezone: string;
  language: string;
  defaultTheme: 'light' | 'dark' | 'system';
  debugMode: boolean;
  logLevel: 'trace' | 'debug' | 'info' | 'warn' | 'error';
}

export interface ServerSettings {
  port: number;
  host: string;
  workers: number;
  timeout: number;
  keepAlive: number;
  maxConnections: number;
  tlsEnabled: boolean;
  tlsCert?: string;
  tlsKey?: string;
  corsEnabled: boolean;
  corsOrigins: string[];
  compressionEnabled: boolean;
}

export interface DatabaseSettings {
  type: DatabaseType;
  host: string;
  port: number;
  name: string;
  username: string;
  password: string;
  connectionString?: string;
  poolSize: number;
  poolTimeout: number;
  sslEnabled: boolean;
  sslMode?: 'disable' | 'require' | 'verify-ca' | 'verify-full';
}

export type DatabaseType = 'postgres' | 'mysql' | 'sqlite' | 'mongodb';

export interface SecuritySettings {
  jwtSecret: string;
  jwtExpiry: number;
  sessionTimeout: number;
  maxLoginAttempts: number;
  lockoutDuration: number;
  passwordMinLength: number;
  passwordRequireSpecial: boolean;
  passwordRequireNumber: boolean;
  passwordRequireUppercase: boolean;
  twoFactorEnabled: boolean;
  encryptionEnabled: boolean;
  encryptionKey?: string;
}

export interface NotificationSettings {
  emailEnabled: boolean;
  emailProvider: 'smtp' | 'sendgrid' | 'aws_ses';
  emailHost?: string;
  emailPort?: number;
  emailUsername?: string;
  emailPassword?: string;
  emailFrom?: string;
  emailRecipients: string[];
  slackEnabled: boolean;
  slackWebhook?: string;
  slackChannel?: string;
  discordEnabled: boolean;
  discordWebhook?: string;
  webhookEnabled: boolean;
  webhookUrl?: string;
  webhookEvents: string[];
}

export interface ApiSettings {
  baseUrl: string;
  version: string;
  rateLimitEnabled: boolean;
  rateLimitDefault: number;
  rateLimitWindow: number;
  requestTimeout: number;
  maxRequestSize: number;
  allowAnonymous: boolean;
  defaultAuthType: 'none' | 'api_key' | 'jwt' | 'oauth';
  apiKeysEnabled: boolean;
  apiKeyPrefix: string;
  apiKeyExpiryDays: number;
}

export interface ChatHistorySettings {
  enabled: boolean;
  storeType: ChatHistoryStoreType;
  retentionDays: number;
  maxConversations: number;
  maxMessagesPerConversation: number;
  compression: boolean;
  encryption: boolean;

  // File-based settings
  filePath?: string;
  directory?: string;

  // Database settings
  dbHost?: string;
  dbPort?: number;
  dbName?: string;
  dbUsername?: string;
  dbPassword?: string;
  dbConnectionString?: string;

  // JSONL specific
  jsonlMaxFileSize?: number;
  jsonlMaxFiles?: number;

  // SQLite specific
  sqlitePath?: string;
  sqliteWALEnabled?: boolean;

  // Postgres specific
  postgresPoolSize?: number;

  // Turso specific
  tursoDatabaseUrl?: string;
  tursoAuthToken?: string;
}


export interface MCPSettings {
  enabled: boolean;
  globalTimeout: number;
  maxConcurrentCalls: number;
  retryAttempts: number;
  defaultStdioServer?: string;
  defaultHttpServer?: string;
  toolsEnabled: boolean;
  resourcesEnabled: boolean;
  promptsEnabled: boolean;
}

export interface PricingSettings {
  enabled: boolean;
  defaultCurrency: string;
  priceMultiplier: number;
  freeTierEnabled: boolean;
  freeTierRequests: number;
  freeTierTokens: number;
  billingCycle: 'monthly' | 'yearly';
  invoiceEnabled: boolean;
  taxEnabled: boolean;
  taxRate: number;
  supportedCurrencies: string[];
}

export interface BedrockSettings {
  enabled: boolean;
  defaultRegion: string;
  credentialsType: 'static' | 'iam_role' | 'ec2_role' | 'ecs_task' | 'lambda';
  accessKeyId?: string;
  secretAccessKey?: string;
  profile?: string;
  useInferenceProfiles: boolean;
  guardrailsEnabled: boolean;
  knowledgeBasesEnabled: boolean;
  agentsEnabled: boolean;
}

export interface SettingsSection {
  id: string;
  label: string;
  icon?: string;
  fields: SettingField[];
}

export interface SettingField {
  id: string;
  name: string;
  type: SettingFieldType;
  label: string;
  description?: string;
  required: boolean;
  defaultValue?: unknown;
  value?: unknown;
  options?: SettingOption[];
  validation?: SettingValidation;
  visible?: (settings: Settings) => boolean;
  enabled?: (settings: Settings) => boolean;
}

export type SettingFieldType =
  | 'text'
  | 'number'
  | 'boolean'
  | 'select'
  | 'multiselect'
  | 'password'
  | 'textarea'
  | 'json'
  | 'array'
  | 'object';

export interface SettingOption {
  label: string;
  value: string | number | boolean;
}

export interface SettingValidation {
  min?: number;
  max?: number;
  minLength?: number;
  maxLength?: number;
  pattern?: string;
  custom?: string;
}

export interface SettingsExport {
  version: string;
  exportedAt: string;
  settings: Settings;
}

export interface SettingsImport {
  settings: Settings;
  merge?: boolean;
  conflictResolution?: 'skip' | 'overwrite' | 'fail';
}

export interface SettingsBackup {
  id: string;
  version: string;
  createdAt: string;
  description?: string;
  settings: Settings;
}

export interface SettingsValidationResult {
  valid: boolean;
  errors: SettingsValidationError[];
  warnings: SettingsValidationWarning[];
}

export interface SettingsValidationError {
  field: string;
  message: string;
  code: string;
}

export interface SettingsValidationWarning {
  field: string;
  message: string;
  code: string;
}
