# Amazon Bedrock Support

Routiium supports both Amazon Bedrock request families:

- `bedrock`: the legacy `bedrock-runtime` InvokeModel adapter for native provider payloads.
- `bedrock_mantle`: the OpenAI-compatible Bedrock Mantle endpoint for Responses and Chat Completions.

Use `bedrock_mantle` for OpenAI models such as `openai.gpt-5.6-luna`. The Mantle path preserves
the OpenAI request and response schemas instead of converting through the legacy InvokeModel
provider adapters. API compatibility is model-specific: GPT-5.6 Luna supports the Responses API,
so use `/v1/responses` for that model. Mantle Chat Completions routing remains available for models
that advertise Chat Completions support.

## Bedrock Mantle configuration

Set the AWS region and either an Amazon Bedrock API key or an AWS credential chain:

```bash
export AWS_REGION=us-east-1
export AWS_BEARER_TOKEN_BEDROCK="<your-bedrock-api-key>"
```

When the bearer token is absent, Routiium signs Mantle requests with the standard AWS SDK
credential chain. This includes environment credentials, shared profiles, and supported workload
identity providers.

Configure a GPT-5.6 route with the exact Mantle model ID and base URL:

```bash
export ROUTIIUM_BACKENDS="prefix=openai.gpt-5.6-,base=https://bedrock-mantle.us-east-1.api.aws/openai/v1,key_env=AWS_BEARER_TOKEN_BEDROCK,mode=bedrock_mantle"
export ROUTIIUM_MANAGED_MODE=1
```

The route matches `openai.gpt-5.6-luna`, `openai.gpt-5.6-terra`, and `openai.gpt-5.6-sol`.
Change the region in the base URL when deploying outside `us-east-1`.

For a Responses request, call Routiium normally:

```bash
curl http://127.0.0.1:8088/v1/responses \
  -H "Authorization: Bearer <routiium-api-key>" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "openai.gpt-5.6-luna",
    "input": [{"role": "user", "content": "Reply with OK."}],
    "store": false
  }'
```

Routiium forwards this to the Mantle `/openai/v1/responses` endpoint. Chat Completions requests
are forwarded to `/openai/v1/chat/completions`.

## Model discovery

Bedrock has separate model catalogs. The AWS CLI command below queries the legacy
`bedrock-runtime` catalog and does not prove Mantle model availability:

```bash
aws bedrock list-foundation-models --by-provider OpenAI
```

Use the Mantle OpenAI-compatible catalog instead:

```bash
curl "https://bedrock-mantle.${AWS_REGION}.api.aws/openai/v1/models" \
  -H "Authorization: Bearer ${AWS_BEARER_TOKEN_BEDROCK}"
```

The model card for GPT-5.6 Luna specifies the exact ID `openai.gpt-5.6-luna` and the
`/openai/v1` base path used by the route above.

## Legacy Bedrock configuration

Keep using `mode=bedrock` for models that require native InvokeModel translation. Configure the
legacy route with a `bedrock://` backend or the existing Bedrock profile. Do not use the legacy
adapter for GPT-5.6 Mantle models.

## Operational notes

- Keep Bedrock credentials on the server; clients should use Routiium API keys.
- Prefer managed mode and admin-token protection for production deployments.
- Validate the model ID, region, IAM permissions, and Mantle quota before rollout.
