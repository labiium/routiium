# Routiium Analytics System

## Overview

The analytics system provides comprehensive tracking and analysis capabilities for all API requests processed by routiium. It captures detailed metrics about requests, responses, performance, authentication, routing, **token usage, and costs**. When integrated with a pricing configuration, the system automatically calculates and tracks costs for all requests, enabling detailed cost analysis and budgeting.

## Architecture

### Components

1. **Analytics Module** (`src/analytics.rs`)
   - Core data models and storage backends
   - Support for Redis, Sled, and in-memory storage
   - Event recording, querying, and aggregation

2. **Analytics Middleware** (`src/analytics_middleware.rs`)
   - Request/response capture framework
   - Context propagation through request lifecycle
   - Automatic metric collection

3. **Analytics Endpoints** (`src/server.rs`)
   - REST API for querying and exporting analytics
   - JSON and CSV export formats
   - Real-time statistics

### Storage Backends

#### JSONL File (Default)
- Append-only JSONL log stored at `data/analytics.jsonl` by default
- Simple to inspect with tools like `jq` or import into external systems
- Parent directories are created automatically
- Data persists until you clear the file via the `/analytics/clear` endpoint or by removing the file
- **Best for:** Small to medium deployments, easy debugging, external tool integration
- **Performance:** Fast writes, slower queries on large datasets

Configuration:
```bash
# Optional override
export ROUTIIUM_ANALYTICS_JSONL_PATH=/var/log/routiium/analytics.jsonl
```

#### Redis (Recommended for Production)
- Persistent storage with automatic expiration (TTL)
- Efficient time-based range queries using sorted sets
- Model and endpoint indexing for fast filtering
- Scales horizontally
- **Best for:** Production deployments, high-volume traffic, distributed systems
- **Performance:** Fast reads and writes, excellent for time-series queries

Configuration:
```bash
export ROUTIIUM_ANALYTICS_REDIS_URL=redis://localhost:6379
export ROUTIIUM_ANALYTICS_TTL_SECONDS=2592000  # 30 days
```

#### Sled (Embedded Database)
- Single-file embedded database
- Good for single-server deployments
- No external dependencies
- Automatic persistence
- **Best for:** Single-server deployments without Redis
- **Performance:** Good balance of speed and simplicity

Configuration:
```bash
export ROUTIIUM_ANALYTICS_SLED_PATH=./analytics.db
export ROUTIIUM_ANALYTICS_TTL_SECONDS=2592000
```

#### Memory (Development Only)
- Fast, in-memory storage
- Limited by available RAM
- Data lost on restart
- Automatic size limiting
- **Best for:** Development, testing, CI/CD pipelines
- **Performance:** Fastest, but not persistent

Configuration:
```bash
export ROUTIIUM_ANALYTICS_FORCE_MEMORY=true
export ROUTIIUM_ANALYTICS_MAX_EVENTS=10000
```

## Data Model

### AnalyticsEvent

Each event captures a complete request/response cycle:

```rust
pub struct AnalyticsEvent {
    pub id: String,                    // Unique event UUID
    pub timestamp: u64,                // Unix timestamp (seconds)
    pub request: RequestMetadata,
    pub response: Option<ResponseMetadata>,
    pub performance: PerformanceMetrics,
    pub auth: AuthMetadata,
    pub routing: RoutingMetadata,
}
```

### RequestMetadata
- `endpoint`: API path (e.g., "/v1/chat/completions")
- `method`: HTTP method
- `model`: Model name requested
- `stream`: Whether streaming was requested
- `size_bytes`: Request payload size
- `message_count`: Number of messages in request
- `input_tokens`: Total input tokens (if available)
- `user_agent`: Client user agent
- `client_ip`: Client IP address (from X-Forwarded-For or connection)

### ResponseMetadata
- `status_code`: HTTP status code
- `size_bytes`: Response size
- `output_tokens`: Total output tokens (if available)
- `success`: Boolean success flag
- `error_message`: Error description if failed

### PerformanceMetrics
- `duration_ms`: Total request duration in milliseconds
- `ttfb_ms`: Time to first byte (for streaming)
- `upstream_duration_ms`: Upstream request time
- `tokens_per_second`: Output tokens divided by duration (for performance analysis)

### TokenUsage (Optional, when available)
Detailed token breakdown from the provider response:
- `prompt_tokens`: Input/prompt tokens consumed
- `completion_tokens`: Output/completion tokens generated
- `total_tokens`: Sum of prompt and completion tokens
- `cached_tokens`: Tokens served from cache (if applicable, e.g., OpenAI prompt caching)
- `reasoning_tokens`: Extended reasoning tokens (for o1/o3 models)

### CostInfo (Optional, when pricing config is loaded)
Calculated cost information based on token usage and model pricing:
- `input_cost`: Cost for input tokens (USD)
- `output_cost`: Cost for output tokens (USD)
- `cached_cost`: Cost for cached tokens (USD, if applicable)
- `total_cost`: Sum of all costs (USD)
- `currency`: Currency code (e.g., "USD")
- `pricing_model`: Model name used for pricing lookup

### AuthMetadata
- `authenticated`: Authentication status
- `api_key_id`: Hashed API key identifier
- `api_key_label`: Human-readable label
- `auth_method`: Authentication method used

### RoutingMetadata
- `backend`: Backend provider (OpenAI, Anthropic, etc.)
- `upstream_mode`: "chat", "responses", or "bedrock"
- `mcp_enabled`: Whether MCP was used
- `mcp_servers`: List of MCP servers invoked
- `system_prompt_applied`: System prompt injection flag

## Cost Tracking Integration

### Enabling Cost Tracking

To enable automatic cost calculation, create a pricing configuration file:

**pricing.json:**
```json
{
  "models": {
    "gpt-4o": {
      "input_per_million": 2.50,
      "output_per_million": 10.00,
      "cached_per_million": 1.25,
      "reasoning_per_million": null
    },
    "gpt-4o-mini": {
      "input_per_million": 0.150,
      "output_per_million": 0.600,
      "cached_per_million": 0.075,
      "reasoning_per_million": null
    },
    "o1": {
      "input_per_million": 15.00,
      "output_per_million": 60.00,
      "cached_per_million": 7.50,
      "reasoning_per_million": 60.00
    }
  },
  "default": {
    "input_per_million": 1.00,
    "output_per_million": 2.00,
    "cached_per_million": 0.50,
    "reasoning_per_million": null
  }
}
```

**Configuration:**
```bash
export ROUTIIUM_PRICING_CONFIG=/path/to/pricing.json
```

### Cost Calculation

When pricing is configured:
1. Analytics middleware extracts token usage from provider responses
2. Costs are calculated using the formula: `(tokens / 1,000,000) × price_per_million`
3. All costs are tracked in USD (or configured currency)
4. Costs are aggregated by model, user, and time period

**Supported token types:**
- **Input tokens**: Standard prompt/input tokens
- **Output tokens**: Generated completion tokens
- **Cached tokens**: Tokens served from provider cache (50% discount on OpenAI)
- **Reasoning tokens**: Extended reasoning for o1/o3 models (billed separately)

### Cost Queries

Query costs using aggregate endpoint:
```bash
# Total cost over last 24 hours
curl "http://localhost:8088/analytics/aggregate?start=$(date -d '24 hours ago' +%s)&end=$(date +%s)" | \
  jq '.total_cost, .cost_by_model'

# Cost per model
curl "http://localhost:8088/analytics/aggregate" | \
  jq '.cost_by_model'
```

## API Endpoints

### GET /analytics/stats

Returns current analytics system statistics, including cost information.

**Response:**
```json
{
  "total_events": 1542,
  "backend_type": "redis",
  "ttl_seconds": 2592000,
  "max_events": null,
  "total_cost": 125.45,
  "total_input_tokens": 1500000,
  "total_output_tokens": 750000,
  "total_cached_tokens": 250000,
  "total_reasoning_tokens": 0,
  "avg_tokens_per_second": 325.4
}
```

### GET /analytics/events

Query individual events with optional time range and limit.

**Query Parameters:**
- `start` (optional): Start timestamp (unix seconds, default: now - 1 hour)
- `end` (optional): End timestamp (unix seconds, default: now)
- `limit` (optional): Maximum events to return

**Response:**
```json
{
  "events": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "timestamp": 1704067200,
      "request": {
        "endpoint": "/v1/chat/completions",
        "method": "POST",
        "model": "gpt-4o",
        "stream": false,
        "size_bytes": 256,
        "message_count": 3,
        "input_tokens": 42,
        "user_agent": "curl/7.64.1",
        "client_ip": "192.168.1.100"
      },
      "response": {
        "status_code": 200,
        "size_bytes": 512,
        "output_tokens": 128,
        "success": true,
        "error_message": null
      },
      "performance": {
        "duration_ms": 1247,
        "ttfb_ms": null,
        "upstream_duration_ms": 1200,
        "tokens_per_second": 102.5
      },
      "token_usage": {
        "prompt_tokens": 42,
        "completion_tokens": 128,
        "total_tokens": 170,
        "cached_tokens": 20,
        "reasoning_tokens": null
      },
      "cost": {
        "input_cost": 0.0000063,
        "output_cost": 0.0000768,
        "cached_cost": 0.0000015,
        "total_cost": 0.0000846,
        "currency": "USD",
        "pricing_model": "gpt-4o-mini"
      },
      "auth": {
        "authenticated": true,
        "api_key_id": "key_abc123",
        "api_key_label": "production-key",
        "auth_method": "bearer"
      },
      "routing": {
        "backend": "openai",
        "upstream_mode": "chat",
        "mcp_enabled": false,
        "mcp_servers": [],
        "system_prompt_applied": true
      }
    }
  ],
  "count": 1,
  "start": 1704067200,
  "end": 1704153600
}
```

### GET /analytics/aggregate

Get aggregated metrics over a time period.

**Query Parameters:**
- `start` (optional): Start timestamp (default: now - 1 hour)
- `end` (optional): End timestamp (default: now)

**Response:**
```json
{
  "total_requests": 1542,
  "successful_requests": 1523,
  "failed_requests": 19,
  "total_input_tokens": 45230,
  "total_output_tokens": 89441,
  "total_cached_tokens": 12500,
  "total_reasoning_tokens": 0,
  "avg_duration_ms": 1247.3,
  "avg_tokens_per_second": 325.4,
  "total_cost": 125.45,
  "cost_by_model": {
    "gpt-4o": 98.32,
    "gpt-4o-mini": 27.13
  },
  "models_used": {
    "gpt-4o": 892,
    "gpt-4o-mini": 650
  },
  "endpoints_hit": {
    "/v1/chat/completions": 892,
    "/v1/responses": 650
  },
  "backends_used": {
    "openai": 1542
  },
  "period_start": 1704067200,
  "period_end": 1704153600
}
```

### GET /analytics/export

Export analytics data in JSON or CSV format.

**Query Parameters:**
- `start` (optional): Start timestamp (default: now - 24 hours)
- `end` (optional): End timestamp (default: now)
- `format` (optional): "json" or "csv" (default: "json")

**CSV Columns:**
- id
- timestamp
- endpoint
- method
- model
- stream
- status_code
- success
- duration_ms
- ttfb_ms
- tokens_per_second
- input_tokens (from token_usage)
- output_tokens (from token_usage)
- cached_tokens (from token_usage)
- reasoning_tokens (from token_usage)
- input_cost (from cost)
- output_cost (from cost)
- cached_cost (from cost)
- total_cost (from cost)
- backend
- upstream_mode
- api_key_id
- api_key_label

**Response Headers:**
- `Content-Type`: application/json or text/csv
- `Content-Disposition`: attachment with filename

### POST /analytics/clear

Clear all analytics data from storage.

**Response:**
```json
{
  "success": true,
  "message": "Analytics data cleared"
}
```

## Use Cases

### Cost Tracking & Budgeting

**Monitor actual costs** (when pricing config is loaded):
```bash
# Get total costs and breakdown by model
curl "http://localhost:8088/analytics/aggregate?start=1704067200&end=1704153600" | \
  jq '{
    total_cost,
    cost_by_model,
    total_input_tokens,
    total_output_tokens,
    total_cached_tokens,
    total_reasoning_tokens
  }'
```

**Track cost per user** (when using managed mode):
```bash
# Export and filter by API key
curl "http://localhost:8088/analytics/export?format=csv&start=1704067200" -o analytics.csv
grep "key_abc123" analytics.csv | awk -F',' '{sum+=$20} END {print "Total: $"sum}'
```

**Monitor token efficiency**:
```bash
# Average tokens per second by model
curl "http://localhost:8088/analytics/aggregate" | \
  jq '.avg_tokens_per_second'
```

**Cost projection**:
```bash
# Calculate hourly rate and project monthly
curl "http://localhost:8088/analytics/aggregate?start=$(date -d '1 hour ago' +%s)" | \
  jq '.total_cost * 24 * 30'  # Hourly × 24 × 30
```

### Performance Monitoring
Track request latency, throughput, and identify slow endpoints:
```bash
# Average duration and tokens per second
curl "http://localhost:8088/analytics/aggregate" | \
  jq '{avg_duration_ms, avg_tokens_per_second}'

# Find slow requests
curl "http://localhost:8088/analytics/events?limit=1000" | \
  jq '.events[] | select(.performance.duration_ms > 5000) | 
    {id, model: .request.model, duration: .performance.duration_ms}'
```

### Usage Analytics
Understand which models and endpoints are most popular:
```bash
curl "http://localhost:8088/analytics/aggregate" | \
  jq '.models_used, .endpoints_hit'
```

### Error Analysis
Identify failed requests and error patterns:
```bash
curl "http://localhost:8088/analytics/events?limit=1000" | \
  jq '.events[] | select(.response.success == false)'
```

### Data Export for External Tools
Export to CSV for analysis in Excel, Tableau, or other tools:
```bash
curl "http://localhost:8088/analytics/export?format=csv&start=1704067200" -o analytics.csv
```

## Integration Examples

### Prometheus Metrics
You can poll the aggregate endpoint and convert to Prometheus format:
```python
import requests
import time

def get_metrics():
    now = int(time.time())
    hour_ago = now - 3600
    resp = requests.get(f"http://localhost:8088/analytics/aggregate?start={hour_ago}&end={now}")
    data = resp.json()
    
    print(f"# HELP routiium_requests_total Total requests")
    print(f"# TYPE routiium_requests_total counter")
    print(f"routiium_requests_total {data['total_requests']}")
    
    print(f"# HELP routiium_tokens_input_total Total input tokens")
    print(f"# TYPE routiium_tokens_input_total counter")
    print(f"routiium_tokens_input_total {data['total_input_tokens']}")
    
    print(f"# HELP routiium_tokens_output_total Total output tokens")
    print(f"# TYPE routiium_tokens_output_total counter")
    print(f"routiium_tokens_output_total {data['total_output_tokens']}")
```

### Grafana Dashboard
Create time-series visualizations by querying aggregated data at regular intervals.

### Cost Calculation & Reporting

**Real-time cost dashboard** (when pricing config is enabled):
```python
import requests
from datetime import datetime, timedelta

def get_cost_report(hours=24):
    """Get detailed cost report for last N hours"""
    now = int(datetime.now().timestamp())
    start = int((datetime.now() - timedelta(hours=hours)).timestamp())
    
    resp = requests.get(
        f"http://localhost:8088/analytics/aggregate",
        params={"start": start, "end": now}
    )
    data = resp.json()
    
    print(f"Cost Report - Last {hours} hours")
    print("=" * 60)
    print(f"Total Cost: ${data['total_cost']:.4f}")
    print(f"Total Requests: {data['total_requests']}")
    print(f"Average Cost per Request: ${data['total_cost'] / data['total_requests']:.6f}")
    print()
    
    print("Cost by Model:")
    for model, cost in sorted(data['cost_by_model'].items(), key=lambda x: x[1], reverse=True):
        requests = data['models_used'][model]
        avg_cost = cost / requests
        print(f"  {model:30} ${cost:8.4f} ({requests:5} req, ${avg_cost:.6f}/req)")
    print()
    
    print("Token Usage:")
    print(f"  Input tokens:     {data['total_input_tokens']:,}")
    print(f"  Output tokens:    {data['total_output_tokens']:,}")
    print(f"  Cached tokens:    {data['total_cached_tokens']:,}")
    print(f"  Reasoning tokens: {data['total_reasoning_tokens']:,}")
    print()
    
    print("Performance:")
    print(f"  Avg duration:     {data['avg_duration_ms']:.1f} ms")
    print(f"  Avg throughput:   {data['avg_tokens_per_second']:.1f} tokens/sec")

get_cost_report(hours=24)
```

**Budget alerts**:
```python
def check_budget_alert(budget_usd=100.0, period_hours=24):
    """Alert if costs exceed budget"""
    now = int(datetime.now().timestamp())
    start = int((datetime.now() - timedelta(hours=period_hours)).timestamp())
    
    resp = requests.get(
        f"http://localhost:8088/analytics/aggregate",
        params={"start": start, "end": now}
    )
    data = resp.json()
    
    if data['total_cost'] > budget_usd:
        print(f"⚠️  BUDGET ALERT: ${data['total_cost']:.2f} exceeds ${budget_usd:.2f}")
        return True
    else:
        print(f"✓ Budget OK: ${data['total_cost']:.2f} / ${budget_usd:.2f}")
        return False
```

**Cost per user tracking**:
```python
def user_costs():
    """Get costs per API key"""
    resp = requests.get("http://localhost:8088/analytics/events?limit=10000")
    data = resp.json()
    
    user_costs = {}
    for event in data['events']:
        if event.get('cost'):
            user = event['auth']['api_key_label'] or event['auth']['api_key_id']
            user_costs[user] = user_costs.get(user, 0) + event['cost']['total_cost']
    
    print("Cost per User:")
    for user, cost in sorted(user_costs.items(), key=lambda x: x[1], reverse=True):
        print(f"  {user:30} ${cost:.4f}")

user_costs()
```

## Best Practices

1. **Set Appropriate TTL**: Configure TTL based on your compliance and storage requirements
   - Development: 7 days (604800 seconds)
   - Production: 30-90 days (2592000-7776000 seconds)
   - Compliance-driven: Adjust based on data retention policies

2. **Use Redis for Production**: Redis provides the best performance and reliability for production workloads
   - Supports clustering for horizontal scaling
   - Fast time-series queries
   - Automatic TTL management

3. **Regular Exports**: Set up scheduled exports for long-term archival and external analysis:
   ```bash
   # Daily export at midnight
   0 0 * * * curl "http://localhost:8088/analytics/export?format=csv" -o "/backups/analytics-$(date +\%Y-\%m-\%d).csv"
   
   # Weekly cost report
   0 0 * * 0 python3 /scripts/weekly_cost_report.py
   ```

4. **Monitor Storage Usage**: Check analytics stats regularly to ensure storage isn't growing unbounded
   ```bash
   # Check total events and estimated storage
   curl "http://localhost:8088/analytics/stats" | jq '{total_events, backend_type, total_cost}'
   ```

5. **Enable Cost Tracking**: Always configure pricing for accurate cost monitoring
   ```bash
   export ROUTIIUM_PRICING_CONFIG=/path/to/pricing.json
   ```

6. **Filter Sensitive Data**: Analytics intentionally doesn't store message content or full API keys
   - Only hashed key IDs and labels are stored
   - No message payloads are captured
   - IP addresses can be anonymized via reverse proxy

7. **Set Budget Alerts**: Implement monitoring for cost thresholds
   ```bash
   # Example: Alert if hourly costs exceed $10
   */30 * * * * /scripts/check_budget.sh
   ```

8. **Optimize Token Usage**: Monitor tokens_per_second to identify performance issues
   ```bash
   # Find models with low throughput
   curl "http://localhost:8088/analytics/events?limit=1000" | \
     jq '.events[] | select(.performance.tokens_per_second < 100) | 
       {model: .request.model, tps: .performance.tokens_per_second}'
   ```

## Privacy and Security

The analytics system is designed with privacy in mind:

- **No message content**: Only metadata is stored, never actual message text
- **Hashed API keys**: Only key IDs and labels are stored, not the actual keys
- **IP anonymization**: Consider using a reverse proxy to strip client IPs if needed
- **Automatic expiration**: TTL ensures data doesn't persist indefinitely
- **Clear endpoint**: Ability to delete all analytics data on demand

## Performance Considerations

- **Redis indexing**: Events are indexed by timestamp, model, and endpoint for fast queries
- **Batch exports**: Large exports may take time; use appropriate time ranges
- **Memory limits**: In memory mode, old events are automatically pruned
- **Async recording**: Analytics recording is non-blocking and won't slow down requests
- **Cost calculation**: Minimal overhead (~1-2ms) when pricing config is loaded
- **Token extraction**: Automatically parses token usage from provider responses

## Pricing Configuration Reference

### Pricing File Format

```json
{
  "models": {
    "model-name": {
      "input_per_million": 2.50,
      "output_per_million": 10.00,
      "cached_per_million": 1.25,
      "reasoning_per_million": null
    }
  },
  "default": {
    "input_per_million": 1.00,
    "output_per_million": 2.00,
    "cached_per_million": 0.50,
    "reasoning_per_million": null
  }
}
```

### Pricing Lookup Logic

1. Exact model name match (e.g., "gpt-4o-mini-2024-07-18")
2. Fallback to prefix match (e.g., "gpt-4o-mini")
3. Use default pricing if no match found
4. Skip cost calculation if pricing unavailable

### Updating Pricing

Pricing can be updated without restarting:
```bash
# Edit pricing.json, then reload
curl -X POST http://localhost:8088/reload/all
```

Or use dynamic pricing updates (if using Router integration):
- Router can return cost information in RoutePlan
- Analytics will use Router-provided costs when available

### Cost Accuracy

**Important**: Costs are estimates based on token counts and pricing configuration:
- Token counts come from provider responses (accurate)
- Pricing must match your actual provider pricing
- Some providers offer volume discounts not reflected in base pricing
- Cached token discounts are model-specific
- Always reconcile with provider billing statements

**Best practice**: Export analytics monthly and compare with provider invoices to validate accuracy.
