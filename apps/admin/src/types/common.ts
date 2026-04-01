// Common types used across the admin panel

export interface BaseEntity {
  id: string;
  created_at?: string;
  updated_at?: string;
}

export interface Timestamp {
  createdAt: string;
  updatedAt?: string;
}

export interface ApiResponse<T = unknown> {
  success: boolean;
  data?: T;
  message?: string;
  error?: string;
}

export interface PaginatedResponse<T = unknown> {
  data: T[];
  total: number;
  page: number;
  pageSize: number;
  totalPages: number;
}

export interface ErrorResponse {
  error: string;
  message: string;
  code?: string;
  details?: Record<string, unknown>;
}

export interface FilterParams {
  search?: string;
  page?: number;
  pageSize?: number;
  sortBy?: string;
  sortOrder?: 'asc' | 'desc';
}

export interface StatusInfo {
  status: 'active' | 'inactive' | 'pending' | 'suspended';
  statusText?: string;
}

export interface PaginationInfo {
  page: number;
  pageSize: number;
  total: number;
  totalPages: number;
}

export type SortOrder = 'asc' | 'desc';

export interface QueryParams {
  [key: string]: string | number | boolean | undefined;
}
