// MCP (Model Context Protocol) types for routiium

export interface MCPConfig {
  id: string;
  name: string;
  enabled: boolean;
  servers: MCPServer[];
  globalTimeout: number;
  maxConcurrentCalls: number;
  retryAttempts: number;
  createdAt: string;
  updatedAt: string;
}

export interface MCPServer {
  id: string;
  name: string;
  type: MCPServerType;
  enabled: boolean;
  config: MCPServerConfig;
  auth?: MCPAuth;
  tools?: MCPTool[];
  resources?: MCPResource[];
  prompts?: MCPPrompt[];
  status?: MCPConnectionStatus;
  lastChecked?: string;
}

export type MCPServerType = 'stdio' | 'http' | 'sse' | 'websocket';

export interface MCPServerConfig {
  // STDIO server config
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  cwd?: string;

  // HTTP/SSE/WebSocket server config
  url?: string;
  headers?: Record<string, string>;
  timeout?: number;

  // General config
  autoReconnect?: boolean;
  maxRetries?: number;
}

export interface MCPAuth {
  type: 'none' | 'api_key' | 'bearer' | 'basic';
  credentials?: Record<string, string>;
}

export interface MCPTool {
  id: string;
  name: string;
  description: string;
  inputSchema: MCPSchema;
  enabled: boolean;
}

export interface MCPResource {
  id: string;
  uri: string;
  name: string;
  description?: string;
  mimeType?: string;
  enabled: boolean;
}

export interface MCPPrompt {
  id: string;
  name: string;
  description: string;
  arguments?: MCPArgument[];
  enabled: boolean;
}

export interface MCPSchema {
  type: 'object' | 'string' | 'number' | 'boolean' | 'array';
  properties?: Record<string, MCPProperty>;
  required?: string[];
  additionalProperties?: boolean;
}

export interface MCPProperty {
  type: 'string' | 'number' | 'boolean' | 'array' | 'object';
  description?: string;
  default?: unknown;
  enum?: unknown[];
  items?: MCPSchema;
  properties?: Record<string, MCPProperty>;
}

export interface MCPArgument {
  name: string;
  description?: string;
  required: boolean;
  type: 'string' | 'number' | 'boolean' | 'array' | 'object';
}

export type MCPConnectionStatus = 'connected' | 'disconnected' | 'connecting' | 'error';

export interface MCPCall {
  id: string;
  serverId: string;
  toolName: string;
  arguments: Record<string, unknown>;
  startedAt: string;
  completedAt?: string;
  duration?: number;
  success: boolean;
  error?: string;
  result?: unknown;
}

export interface MCPCallLog {
  calls: MCPCall[];
  total: number;
  page: number;
  pageSize: number;
}

export interface MCPStats {
  serverId: string;
  totalCalls: number;
  successfulCalls: number;
  failedCalls: number;
  averageDuration: number;
  lastCall?: string;
}

export interface MCPFilter {
  serverId?: string;
  toolName?: string;
  success?: boolean;
  startDate?: string;
  endDate?: string;
  search?: string;
}

export interface MCPTestRequest {
  serverId: string;
  toolName: string;
  arguments: Record<string, unknown>;
}

export interface MCPTestResponse {
  success: boolean;
  result?: unknown;
  error?: string;
  duration: number;
}
