// AWS Bedrock types for routiium Bedrock integration

export interface BedrockConfig {
  id: string;
  name: string;
  enabled: boolean;
  region: string;
  credentials: BedrockCredentials;
  defaultModel?: string;
  inferenceProfiles?: BedrockInferenceProfile[];
  customModels?: BedrockCustomModel[];
  createdAt: string;
  updatedAt: string;
}

export interface BedrockCredentials {
  type: 'static' | 'iam_role' | 'ec2_role' | 'ecs_task' | 'lambda';
  accessKeyId?: string;
  secretAccessKey?: string;
  region?: string;
  roleArn?: string;
  profile?: string;
}

export interface BedrockInferenceProfile {
  id: string;
  arn: string;
  name: string;
  models: string[];
  maxConcurrentRequests: number;
  status: BedrockProfileStatus;
}

export type BedrockProfileStatus = 'active' | 'inactive' | 'error';

export interface BedrockCustomModel {
  id: string;
  name: string;
  baseModel: string;
  modelArn?: string;
  inferenceMode: BedrockInferenceMode;
  hyperparameters?: Record<string, unknown>;
  enabled: boolean;
}

export type BedrockInferenceMode = 'on_demand' | 'provisioned' | 'inference_profile';

export interface BedrockModel {
  id: string;
  name: string;
  provider: string;
  modelId: string;
  inputModalities: BedrockModality[];
  outputModalities: BedrockModality[];
  maxInputTokens: number;
  maxOutputTokens: number;
  supportsStreaming: boolean;
  supportsSystemPrompt: boolean;
  supportsToolUse: boolean;
  supportsPromptCaching: boolean;
  pricing?: BedrockModelPricing;
}

export type BedrockModality = 'text' | 'image' | 'chat' | 'embeddings' | 'video';

export interface BedrockModelPricing {
  inputPricePer1M: number;
  outputPricePer1M: number;
  currency: string;
}

export interface BedrockRequest {
  modelId: string;
  messages: BedrockMessage[];
  system?: string | BedrockSystemPrompt[];
  inferenceConfig?: BedrockInferenceConfig;
  toolConfig?: BedrockToolConfig;
  additionalModelRequestFields?: Record<string, unknown>;
}

export interface BedrockMessage {
  role: 'user' | 'assistant';
  content: BedrockContent[];
}

export interface BedrockContent {
  type: 'text' | 'image' | 'tool_use' | 'tool_result';
  text?: string;
  image?: BedrockImageContent;
  toolUse?: BedrockToolUse;
  toolResult?: BedrockToolResult;
}

export interface BedrockImageContent {
  format: 'png' | 'jpeg' | 'gif' | 'webp';
  source: {
    bytes?: string;
    s3?: BedrockS3Location;
  };
}

export interface BedrockS3Location {
  bucket: string;
  key: string;
}

export interface BedrockSystemPrompt {
  text: string;
}

export interface BedrockInferenceConfig {
  maxTokens?: number;
  temperature?: number;
  topP?: number;
  stopSequences?: string[];
}

export interface BedrockToolConfig {
  tools: BedrockTool[];
  toolChoice?: BedrockToolChoice;
}

export interface BedrockTool {
  name: string;
  description: string;
  inputSchema: BedrockJsonSchema;
}

export interface BedrockJsonSchema {
  type: 'object';
  properties?: Record<string, BedrockJsonSchemaProperty>;
  required?: string[];
}

export interface BedrockJsonSchemaProperty {
  type: string;
  description?: string;
  enum?: string[];
}

export interface BedrockToolChoice {
  type: 'auto' | 'any' | 'tool';
  name?: string;
}

export interface BedrockToolUse {
  id: string;
  name: string;
  input: Record<string, unknown>;
}

export interface BedrockToolResult {
  toolUseId: string;
  content: BedrockToolResultContent[];
}

export interface BedrockToolResultContent {
  type: 'text';
  text: string;
}

export interface BedrockResponse {
  id: string;
  modelId: string;
  content: BedrockContent[];
  stopReason: BedrockStopReason;
  usage: BedrockUsage;
  metrics?: BedrockMetrics;
}

export type BedrockStopReason = 'end_turn' | 'max_tokens' | 'stop_sequence' | 'tool_use' | 'content_filtered';

export interface BedrockUsage {
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
}

export interface BedrockMetrics {
  latencyMs?: number;
  inferenceTimeMs?: number;
  firstTokenLatencyMs?: number;
}

export interface Bedrock_converseRequest {
  modelId: string;
  messages: BedrockMessage[];
  system?: string;
  inferenceConfig?: BedrockInferenceConfig;
  toolConfig?: BedrockToolConfig;
}

export interface Bedrock_converseResponse {
  id: string;
  modelId: string;
  output: {
    message: BedrockMessage;
  };
  stopReason: BedrockStopReason;
  usage: BedrockUsage;
}

export interface BedrockInvocationLog {
  id: string;
  timestamp: string;
  modelId: string;
  requestId: string;
  inputTokens: number;
  outputTokens: number;
  latencyMs: number;
  status: 'success' | 'error';
  error?: string;
  userId?: string;
  apiKeyId?: string;
}

export interface BedrockStats {
  totalInvocations: number;
  successfulInvocations: number;
  failedInvocations: number;
  totalInputTokens: number;
  totalOutputTokens: number;
  averageLatency: number;
  byModel: BedrockModelStats[];
  byUser: BedrockUserStats[];
}

export interface BedrockModelStats {
  modelId: string;
  invocations: number;
  inputTokens: number;
  outputTokens: number;
  averageLatency: number;
}

export interface BedrockUserStats {
  userId: string;
  invocations: number;
  inputTokens: number;
  outputTokens: number;
}

export interface BedrockFilter {
  modelId?: string;
  userId?: string;
  apiKeyId?: string;
  startDate?: string;
  endDate?: string;
  status?: 'success' | 'error';
}

export interface BedrockGuardrail {
  id: string;
  name: string;
  arn: string;
  version: string;
  status: 'active' | 'inactive';
  config?: BedrockGuardrailConfig;
}

export interface BedrockGuardrailConfig {
  filters?: BedrockContentFilter[];
  sensitiveInformation?: BedrockSensitiveInfoFilter[];
  contextual?: BedrockContextualGroundingFilter[];
}

export interface BedrockContentFilter {
  type: 'hate' | 'insults' | 'sexual' | 'violence' | 'harassment';
  action: 'block' | 'filter' | 'none';
  threshold?: number;
}

export interface BedrockSensitiveInfoFilter {
  type: 'pii' | 'regex' | 'word';
  action: 'block' | 'mask' | 'none';
  config?: Record<string, unknown>;
}

export interface BedrockContextualGroundingFilter {
  enabled: boolean;
  threshold?: number;
}

export interface BedrockKnowledgeBase {
  id: string;
  name: string;
  knowledgeBaseId: string;
  status: 'active' | 'inactive' | 'pending';
  embeddingModel?: string;
  vectorStore?: string;
  createdAt: string;
}

export interface BedrockAgent {
  id: string;
  name: string;
  agentId: string;
  aliasId?: string;
  status: 'active' | 'inactive';
  instruction?: string;
  knowledgeBases?: string[];
  createdAt: string;
  updatedAt: string;
}
