# Routiium Admin Panel

This Vite app is wired to Routiium’s real admin/runtime APIs and does not use synthetic page data.

## What It Maps To

| Page | Backing Routiium endpoints |
| --- | --- |
| Dashboard | `GET /admin/panel/state` |
| API Keys | `GET /keys`, `POST /keys/generate`, `POST /keys/revoke` |
| Routing | `GET /admin/panel/state`, `PUT /admin/panel/routing`, `POST /reload/routing` |
| Rate Limiting | `GET /admin/panel/state`, `POST/PUT/DELETE /admin/rate-limits/policies*`, `POST /admin/rate-limits/default`, `POST/DELETE /admin/rate-limits/keys/{key_id}`, `POST/GET/DELETE /admin/rate-limits/emergency*`, `POST /admin/rate-limits/reload` |
| Analytics | `GET /analytics/stats`, `GET /analytics/aggregate`, `GET /analytics/events`, `GET /analytics/export` |
| System Prompts | `GET /admin/panel/state`, `PUT /admin/panel/system-prompts`, `POST /reload/system_prompt` |
| MCP | `GET /admin/panel/state`, `PUT /admin/panel/mcp`, `POST /reload/mcp` |
| Pricing | `GET /admin/panel/state` |
| Bedrock | `GET /admin/panel/state` |
| Chat History | `GET /admin/panel/state`, `GET /chat_history/conversations`, `GET /chat_history/messages`, `DELETE /chat_history/conversations/{id}`, `POST /chat_history/clear` |
| Principals | `GET /admin/panel/state` |
| Settings | `GET /admin/panel/state` |

## Writable vs Read-Only

- Writable:
  - API keys
  - Rate limit policies, assignments, emergency blocks
  - System prompt config
  - MCP config
  - Local routing config
- Read-only by design:
  - Pricing
  - Bedrock
  - Principals
  - Runtime settings derived from environment

These sections are read-only because Routiium currently exposes runtime inspection for them, not safe live mutation APIs.

## Running It

```bash
cd apps/admin
npm install
npm run dev
```

Or build it:

```bash
npm run build
```

From the repo root:

```bash
npm run admin:install
npm run admin:build
```

## Connecting To Routiium

The header includes:

- `API base URL`
- `Admin bearer token`

Those values are stored in browser local storage and used for every request. If `ROUTIIUM_ADMIN_TOKEN` is configured on the server, enter the matching bearer token here.

Default API base URL is the current browser origin. Override it when the panel is served separately from Routiium.

## Required Server Support

This panel expects the Routiium server to expose:

- Existing admin/runtime routes already documented in the main project README
- Additional panel snapshot/config routes:
  - `GET /admin/panel/state`
  - `PUT /admin/panel/system-prompts`
  - `PUT /admin/panel/mcp`
  - `PUT /admin/panel/routing`

## Notes

- The “Principals” page intentionally represents API-key principals, not a separate user database. Routiium does not maintain first-class human user records.
- Bedrock visibility is detection-based. It reflects runtime mode, routing config, and router catalog data available to Routiium.
