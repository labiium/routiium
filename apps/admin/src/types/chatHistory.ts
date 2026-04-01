// Chat History types for routiium conversation storage

export interface Message {
  id: string;
  role: MessageRole;
  content: string;
  name?: string;
  toolCalls?: ToolCall[];
  toolCallId?: string;
  attachments?: Attachment[];
  metadata?: Record<string, unknown>;
  createdAt: string;
}

export type MessageRole = 'user' | 'assistant' | 'system' | 'tool';

export interface ToolCall {
  id: string;
  type: 'function';
  function: {
    name: string;
    arguments: string;
    output?: string;
  };
}

export interface Attachment {
  id: string;
  fileName: string;
  fileType: string;
  url: string;
  size: number;
}

export interface Conversation {
  id: string;
  apiKeyId: string;
  userId?: string;
  model?: string;
  provider?: string;
  messages: Message[];
  tokenCount?: TokenInfo;
  cost?: ChatCostInfo;
  routing?: RoutingInfo;
  mcp?: MCPInfo[];
  privacyLevel: PrivacyLevel;
  tags?: string[];
  metadata?: Record<string, unknown>;
  createdAt: string;
  updatedAt: string;
  lastMessageAt?: string;
}

export type PrivacyLevel = 'public' | 'private' | 'confidential' | 'restricted';

export interface TokenInfo {
  input: number;
  output: number;
  total: number;
  cached?: number;
  reasoning?: number;
}

export interface ChatCostInfo {
  inputCost: number;
  outputCost: number;
  totalCost: number;
  currency: string;
}

export interface RoutingInfo {
  routeId: string;
  routeName: string;
  model?: string;
  provider?: string;
  latency?: number;
}

export interface MCPInfo {
  server: string;
  tool?: string;
  duration?: number;
  success: boolean;
}

export interface ConversationFilters {
  apiKeyId?: string;
  userId?: string;
  model?: string;
  provider?: string;
  privacyLevel?: PrivacyLevel;
  startDate?: string;
  endDate?: string;
  search?: string;
  tags?: string[];
}

export interface MessageFilters {
  role?: MessageRole;
  search?: string;
  startDate?: string;
  endDate?: string;
}

export interface ChatHistoryStore {
  type: ChatHistoryStoreType;
  config: ChatHistoryConfig;
  connected: boolean;
  stats?: StorageStats;
}

export type ChatHistoryStoreType =
  | 'memory'
  | 'jsonl'
  | 'sqlite'
  | 'postgres'
  | 'turso';

export interface ChatHistoryConfig {
  // Common config
  enabled: boolean;
  storeType: ChatHistoryStoreType;

  // Memory store config
  maxConversations?: number;
  maxMessagesPerConversation?: number;

  // File-based store config (JSONL, SQLite)
  filePath?: string;
  directory?: string;

  // Database store config (Postgres, Turso)
  host?: string;
  port?: number;
  database?: string;
  username?: string;
  password?: string;
  connectionString?: string;

  // General config
  retentionDays?: number;
  compression?: boolean;
  encryption?: boolean;
  batchSize?: number;
}

export interface StorageStats {
  totalConversations: number;
  totalMessages: number;
  totalTokens: number;
  totalSize: number;
  sizeUnit: string;
  lastUpdated: string;
}

export interface ChatHistoryExport {
  conversationIds?: string[];
  format: 'json' | 'jsonl' | 'csv';
  includeMessages: boolean;
  includeMetadata: boolean;
  compress?: boolean;
}

export interface ChatHistoryImport {
  source: string;
  format: 'json' | 'jsonl' | 'csv';
  merge?: boolean;
  conflictResolution?: 'skip' | 'overwrite' | 'fail';
}

export interface ChatHistoryBackup {
  id: string;
  createdAt: string;
  size: number;
  fileUrl?: string;
  storeTypes: ChatHistoryStoreType[];
  status: 'pending' | 'completed' | 'failed';
}
