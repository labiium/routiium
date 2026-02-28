# Production Hardening Checklist

Use this checklist before exposing Routiium beyond a trusted network.

## 1) Secrets and Credentials

- Rotate any credentials that were ever committed or shared in plain text.
- Keep provider/API credentials in a secret manager (Vault, AWS Secrets Manager, Kubernetes Secrets, etc.).
- Do not pass a broad `.env` file into containers; pass only required variables.
- Set `ROUTIIUM_ADMIN_TOKEN` and store it as a secret.

## 2) Admin Surface Protection

Routiium exposes administrative routes:

- `/keys*`
- `/reload/*`
- `/analytics/*`
- `/chat_history/*`

When `ROUTIIUM_ADMIN_TOKEN` is configured, these require:

```http
Authorization: Bearer <ROUTIIUM_ADMIN_TOKEN>
```

Still enforce network-layer restrictions (private subnet, service mesh policy, ingress allowlists).

## 3) Runtime and Container Security

- Run as non-root (already default in Dockerfile).
- Keep root filesystem read-only and mount writable volumes only where needed (`/data`).
- Drop Linux capabilities and enable `no-new-privileges` (already in `docker-compose.yml`).
- Keep `/tmp` as `tmpfs` with `noexec,nosuid`.
- Pin and regularly update base images.

## 4) Auth Mode and Routing Controls

- Explicitly set `ROUTIIUM_MANAGED_MODE` (`managed` or `passthrough`) in production.
- If using Router policies, set `ROUTIIUM_ROUTER_STRICT=true` for fail-closed behavior.
- Use per-provider `key_env` values in routing to avoid accidental fallback key usage.

## 5) Observability and Data Governance

- Persist analytics/chat history to controlled storage (`/data` JSONL or managed DB/Redis).
- Set retention controls (`ROUTIIUM_ANALYTICS_TTL_SECONDS`, chat history TTL settings).
- Treat analytics/chat history as sensitive data; restrict endpoint access and exports.

## 6) Release and Operations

- Gate releases with `cargo test` and at least one live upstream smoke test.
- Add vulnerability scanning for images and dependencies in CI.
- Roll keys/tokens on schedule and after any incident.
