// User types for routiium admin panel

export interface User {
  id: string;
  email: string;
  name: string;
  role: UserRole;
  status: UserStatus;
  avatar?: string;
  apiKeys?: string[];
  settings?: UserSettings;
  metadata?: Record<string, unknown>;
  createdAt: string;
  updatedAt: string;
  lastLoginAt?: string;
}

export type UserRole = 'admin' | 'developer' | 'analyst' | 'viewer' | 'custom';

export type UserStatus = 'active' | 'inactive' | 'suspended' | 'pending';

export interface UserSettings {
  theme?: 'light' | 'dark' | 'system';
  language?: string;
  timezone?: string;
  notifications?: UserNotifications;
  dashboard?: UserDashboardSettings;
}

export interface UserNotifications {
  email?: boolean;
  slack?: boolean;
  inApp?: boolean;
 DigestFrequency?: 'realtime' | 'daily' | 'weekly';
}

export interface UserDashboardSettings {
  defaultView?: string;
  widgets?: string[];
  refreshInterval?: number;
}

export interface CreateUserRequest {
  email: string;
  name: string;
  role?: UserRole;
  apiKeys?: string[];
  settings?: UserSettings;
  sendInvite?: boolean;
}

export interface UpdateUserRequest {
  name?: string;
  role?: UserRole;
  status?: UserStatus;
  settings?: UserSettings;
  metadata?: Record<string, unknown>;
}

export interface UserFilter {
  search?: string;
  role?: UserRole;
  status?: UserStatus;
  createdAfter?: string;
  createdBefore?: string;
}

export interface UserStats {
  totalUsers: number;
  activeUsers: number;
  inactiveUsers: number;
  suspendedUsers: number;
  usersByRole: RoleCount[];
  recentLogins: UserLogin[];
}

export interface RoleCount {
  role: UserRole;
  count: number;
}

export interface UserLogin {
  userId: string;
  email: string;
  timestamp: string;
  ip?: string;
  userAgent?: string;
  success: boolean;
}

export interface UserActivity {
  id: string;
  userId: string;
  action: UserActivityAction;
  resource?: string;
  resourceId?: string;
  metadata?: Record<string, unknown>;
  ip?: string;
  timestamp: string;
}

export type UserActivityAction =
  | 'login'
  | 'logout'
  | 'create'
  | 'update'
  | 'delete'
  | 'view'
  | 'export'
  | 'api_key_create'
  | 'api_key_revoke'
  | 'settings_update';

export interface UserPermissions {
  userId: string;
  permissions: Permission[];
  groups: string[];
}

export interface Permission {
  resource: string;
  actions: string[];
}

export interface UserGroup {
  id: string;
  name: string;
  description?: string;
  members: string[];
  roles: string[];
  createdAt: string;
  updatedAt: string;
}

export interface InviteUserRequest {
  email: string;
  name: string;
  role?: UserRole;
  groupIds?: string[];
  expiresIn?: number; // hours
  message?: string;
}

export interface InviteUserResponse {
  inviteId: string;
  inviteUrl?: string;
  expiresAt: string;
}

export interface UserAuditLog {
  userId: string;
  actions: UserActivity[];
  total: number;
  page: number;
  pageSize: number;
}

export interface BulkUserAction {
  userIds: string[];
  action: 'activate' | 'deactivate' | 'suspend' | 'delete' | 'change_role';
  parameters?: Record<string, unknown>;
}
