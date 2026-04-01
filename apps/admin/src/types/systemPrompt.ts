// System Prompt types for routiium prompt management

export interface SystemPrompt {
  id: string;
  name: string;
  content: string;
  description?: string;
  version: number;
  active: boolean;
  variables?: PromptVariable[];
  tags?: string[];
  createdAt: string;
  updatedAt: string;
  createdBy?: string;
}

export interface PromptVariable {
  name: string;
  type: PromptVariableType;
  description?: string;
  required: boolean;
  defaultValue?: string;
  allowedValues?: string[];
  validation?: PromptValidation;
}

export type PromptVariableType = 'string' | 'number' | 'boolean' | 'array' | 'object';

export interface PromptValidation {
  pattern?: string;
  min?: number;
  max?: number;
  minLength?: number;
  maxLength?: number;
  custom?: string;
}

export interface SystemPromptVersion {
  id: string;
  promptId: string;
  version: number;
  content: string;
  variables?: PromptVariable[];
  changelog?: string;
  createdAt: string;
  createdBy?: string;
}

export interface SystemPromptFilter {
  search?: string;
  active?: boolean;
  tags?: string[];
  createdAfter?: string;
  createdBefore?: string;
  updatedAfter?: string;
  updatedBefore?: string;
}

export interface SystemPromptTest {
  promptId: string;
  variables?: Record<string, unknown>;
  expectedOutput?: string;
}

export interface SystemPromptTestResult {
  success: boolean;
  parsedContent?: string;
  variablesFound?: string[];
  variablesMissing?: string[];
  validationErrors?: string[];
  error?: string;
}

export interface PromptTemplate {
  id: string;
  name: string;
  description: string;
  category: PromptCategory;
  content: string;
  variables?: PromptVariable[];
  examples?: PromptExample[];
  tags?: string[];
  popularity: number;
  createdAt: string;
  updatedAt: string;
}

export type PromptCategory =
  | 'general'
  | 'coding'
  | 'writing'
  | 'analysis'
  | 'conversation'
  | 'agent'
  | 'custom';

export interface PromptExample {
  input: string;
  output: string;
  description?: string;
}

export interface PromptAnalytics {
  promptId: string;
  totalUsages: number;
  successfulUsages: number;
  failedUsages: number;
  averageLatency: number;
  averageTokens: number;
  lastUsed?: string;
  feedback?: PromptFeedback[];
}

export interface PromptFeedback {
  id: string;
  promptId: string;
  userId?: string;
  rating: number; // 1-5
  comment?: string;
  createdAt: string;
}

export interface PromptImport {
  prompts: SystemPrompt[];
  merge?: boolean;
  conflictResolution?: 'skip' | 'overwrite' | 'fail';
}

export interface PromptExport {
  promptIds?: string[];
  format: 'json' | 'yaml' | 'markdown';
  includeVersions?: boolean;
  includeAnalytics?: boolean;
}
